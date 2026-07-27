#![allow(
    clippy::disallowed_methods,
    reason = "this single-threaded CLI intentionally blocks while sampling and controlling capture subprocesses"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use tempfile::Builder;

#[derive(Parser, Debug)]
#[command(name = "omega-sniffer")]
#[command(about = "Capture and inspect network traffic from an independent macOS application")]
struct Args {
    #[command(subcommand)]
    command: SnifferCommand,
}

#[derive(Subcommand, Debug)]
enum SnifferCommand {
    /// Capture traffic for an application that does not integrate with Omega.
    Capture {
        /// A PID, application name, or bundle identifier such as com.googlecode.iterm2.
        #[arg(long)]
        application: String,
        /// Number of seconds to capture.
        #[arg(long, default_value_t = 30)]
        duration: u64,
        /// Artifact path. Pcapng contains packet bytes; JSONL contains unprivileged flow samples.
        #[arg(long)]
        output: PathBuf,
        /// Capture full packets or unprivileged flow metadata.
        #[arg(long, value_enum, default_value_t = CaptureFormat::Pcapng)]
        format: CaptureFormat,
        /// Include child processes launched by the application.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_children: bool,
    },
    /// Produce a compact JSON summary suitable for an agent.
    Inspect {
        #[arg(long)]
        input: PathBuf,
        /// Maximum decoded packets to include for a pcapng artifact.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CaptureFormat {
    Pcapng,
    Jsonl,
}

#[derive(Debug, Clone)]
struct Process {
    pid: u32,
    parent_pid: u32,
    command: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FlowSample {
    timestamp_ms: u128,
    root_pid: u32,
    observed_pids: Vec<u32>,
    process: String,
    pid: u32,
    flow: Option<String>,
    interface: Option<String>,
    state: Option<String>,
    bytes_in: Option<u64>,
    bytes_out: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CaptureManifest {
    application: String,
    root_pid: u32,
    observed_pids: Vec<u32>,
    started_at_ms: u128,
    duration_seconds: u64,
    format: &'static str,
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct JsonlSummary {
    format: &'static str,
    samples: usize,
    processes: Vec<ProcessSummary>,
    first_timestamp_ms: Option<u128>,
    last_timestamp_ms: Option<u128>,
}

#[derive(Debug, Serialize)]
struct ProcessSummary {
    process: String,
    pid: u32,
    flows: Vec<String>,
    maximum_bytes_in: u64,
    maximum_bytes_out: u64,
}

#[derive(Debug, Serialize)]
struct PcapSummary {
    format: &'static str,
    packet_count: u64,
    decoded_lines: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !cfg!(target_os = "macos") {
        bail!("omega-sniffer currently supports macOS only");
    }

    match args.command {
        SnifferCommand::Capture {
            application,
            duration,
            output,
            format,
            include_children,
        } => {
            if duration == 0 {
                bail!("--duration must be greater than zero");
            }
            ensure_output_parent(&output)?;
            let root_pid = resolve_application(&application)?;
            match format {
                CaptureFormat::Pcapng => {
                    capture_packets(&application, root_pid, duration, &output, include_children)
                }
                CaptureFormat::Jsonl => {
                    capture_metadata(&application, root_pid, duration, &output, include_children)
                }
            }
        }
        SnifferCommand::Inspect { input, limit } => inspect(&input, limit),
    }
}

fn ensure_output_parent(output: &Path) -> Result<()> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    Ok(())
}

fn resolve_application(application: &str) -> Result<u32> {
    if let Ok(pid) = application.parse::<u32>() {
        ensure_process_exists(pid)?;
        return Ok(pid);
    }
    if let Some(pid) = lsappinfo_pid(application)? {
        return Ok(pid);
    }

    let bundle_lookup = command_output(
        Command::new("/usr/bin/lsappinfo").args(["find", &format!("bundleID={application}")]),
        "querying LaunchServices",
    )?;
    if bundle_lookup.status.success() {
        let serial_number = String::from_utf8_lossy(&bundle_lookup.stdout);
        let serial_number = serial_number.trim();
        if !serial_number.is_empty()
            && let Some(pid) = lsappinfo_pid(serial_number)?
        {
            return Ok(pid);
        }
    }
    if let Some(pid) = running_bundle_pid(application)? {
        return Ok(pid);
    }

    let application_lowercase = application.to_lowercase();
    let matches = process_list()?
        .into_iter()
        .filter(|process| {
            Path::new(&process.command)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().to_lowercase() == application_lowercase)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [process] => Ok(process.pid),
        [] => bail!(
            "no running application matched {application:?}; pass a PID, application name, or bundle identifier"
        ),
        _ => bail!(
            "multiple processes matched {application:?}: {}; pass a PID",
            matches
                .iter()
                .map(|process| process.pid.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn running_bundle_pid(bundle_identifier: &str) -> Result<Option<u32>> {
    for process in process_list()? {
        let Some(bundle_path) = app_bundle_path(&process.command) else {
            continue;
        };
        let output = command_output(
            Command::new("/usr/bin/mdls")
                .args(["-raw", "-name", "kMDItemCFBundleIdentifier"])
                .arg(&bundle_path),
            "reading a running application's bundle identifier",
        )?;
        if output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim() == bundle_identifier
        {
            return Ok(Some(process.pid));
        }
    }
    Ok(None)
}

fn app_bundle_path(command: &str) -> Option<PathBuf> {
    let app_end = command.find(".app/")? + ".app".len();
    Some(PathBuf::from(&command[..app_end]))
}

fn lsappinfo_pid(application: &str) -> Result<Option<u32>> {
    let output = command_output(
        Command::new("/usr/bin/lsappinfo").args(["info", "-only", "pid", "-app", application]),
        "querying LaunchServices",
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .split(|character: char| !character.is_ascii_digit())
        .find_map(|field| field.parse::<u32>().ok()))
}

fn ensure_process_exists(pid: u32) -> Result<()> {
    if process_list()?.iter().any(|process| process.pid == pid) {
        Ok(())
    } else {
        bail!("process {pid} is not running")
    }
}

fn process_list() -> Result<Vec<Process>> {
    let output = command_output(
        Command::new("/bin/ps").args(["-axo", "pid=,ppid=,comm="]),
        "listing processes",
    )?;
    if !output.status.success() {
        bail!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields
                .next()
                .ok_or_else(|| anyhow!("missing PID in ps output"))?
                .parse()
                .context("invalid PID in ps output")?;
            let parent_pid = fields
                .next()
                .ok_or_else(|| anyhow!("missing parent PID in ps output"))?
                .parse()
                .context("invalid parent PID in ps output")?;
            Ok(Process {
                pid,
                parent_pid,
                command: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

fn related_processes(root_pid: u32, include_children: bool) -> Result<BTreeMap<u32, String>> {
    let processes = process_list()?;
    if !processes.iter().any(|process| process.pid == root_pid) {
        bail!("target application process {root_pid} is no longer running");
    }
    let mut related = BTreeSet::from([root_pid]);
    if include_children {
        loop {
            let previous_length = related.len();
            for process in &processes {
                if related.contains(&process.parent_pid) {
                    related.insert(process.pid);
                }
            }
            if related.len() == previous_length {
                break;
            }
        }
    }
    Ok(processes
        .into_iter()
        .filter(|process| related.contains(&process.pid))
        .map(|process| (process.pid, process.command))
        .collect())
}

fn capture_metadata(
    application: &str,
    root_pid: u32,
    duration_seconds: u64,
    output: &Path,
    include_children: bool,
) -> Result<()> {
    let mut artifact =
        File::create(output).with_context(|| format!("failed to create {}", output.display()))?;
    let started_at_ms = timestamp_ms()?;
    let deadline = Instant::now() + Duration::from_secs(duration_seconds);
    let mut observed_pids = BTreeSet::new();

    while Instant::now() < deadline {
        let processes = related_processes(root_pid, include_children)?;
        observed_pids.extend(processes.keys().copied());
        let nettop_output = run_nettop(&processes)?;
        for sample in parse_nettop(
            &String::from_utf8_lossy(&nettop_output.stdout),
            root_pid,
            &processes,
        )? {
            serde_json::to_writer(&mut artifact, &sample)
                .context("failed to serialize flow sample")?;
            artifact
                .write_all(b"\n")
                .context("failed to write flow sample")?;
        }
        artifact.flush().context("failed to flush flow artifact")?;
        thread::sleep(Duration::from_millis(250));
    }

    let manifest = CaptureManifest {
        application: application.to_string(),
        root_pid,
        observed_pids: observed_pids.into_iter().collect(),
        started_at_ms,
        duration_seconds,
        format: "jsonl-flow-metadata",
        output: output.to_path_buf(),
    };
    write_manifest(output, &manifest)?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

fn run_nettop(processes: &BTreeMap<u32, String>) -> Result<Output> {
    let mut command = Command::new("/usr/bin/nettop");
    command.args([
        "-L",
        "1",
        "-n",
        "-x",
        "-J",
        "interface,state,bytes_in,bytes_out",
    ]);
    let output = command_output(&mut command, "sampling network flows")?;
    if !output.status.success() {
        bail!(
            "nettop failed with {} for PIDs {}: {}",
            output.status,
            processes
                .keys()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }
    Ok(output)
}

fn parse_nettop(
    output: &str,
    root_pid: u32,
    processes: &BTreeMap<u32, String>,
) -> Result<Vec<FlowSample>> {
    let mut samples = Vec::new();
    let mut current_process: Option<(String, u32)> = None;
    let timestamp_ms = timestamp_ms()?;
    let observed_pids = processes.keys().copied().collect::<Vec<_>>();

    for line in output.lines() {
        let fields = line.split(',').collect::<Vec<_>>();
        let Some(first) = fields.first().copied() else {
            continue;
        };
        if first.is_empty() || first == "time" {
            continue;
        }
        let is_process_summary = fields.get(1).is_some_and(|field| field.is_empty())
            && fields.get(2).is_some_and(|field| field.is_empty());
        if is_process_summary
            && let Some((_name, pid)) = first.rsplit_once('.')
            && let Ok(pid) = pid.parse::<u32>()
        {
            current_process = processes
                .get(&pid)
                .map(|command| (process_display_name(command), pid));
            if let Some((process_name, pid)) = &current_process {
                samples.push(flow_sample(
                    timestamp_ms,
                    root_pid,
                    &observed_pids,
                    process_name,
                    *pid,
                    None,
                    &fields,
                ));
            }
            continue;
        }
        if let Some((name, pid)) = &current_process
            && processes.contains_key(pid)
        {
            samples.push(flow_sample(
                timestamp_ms,
                root_pid,
                &observed_pids,
                name,
                *pid,
                Some(first.trim().to_string()),
                &fields,
            ));
        }
    }
    Ok(samples)
}

fn process_display_name(command: &str) -> String {
    Path::new(command)
        .file_name()
        .map(|name| name.to_string_lossy().trim_start_matches('-').to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| command.to_string())
}

fn flow_sample(
    timestamp_ms: u128,
    root_pid: u32,
    observed_pids: &[u32],
    process: &str,
    pid: u32,
    flow: Option<String>,
    fields: &[&str],
) -> FlowSample {
    FlowSample {
        timestamp_ms,
        root_pid,
        observed_pids: observed_pids.to_vec(),
        process: process.to_string(),
        pid,
        flow,
        interface: nonempty_field(fields, 1),
        state: nonempty_field(fields, 2),
        bytes_in: numeric_field(fields, 3),
        bytes_out: numeric_field(fields, 4),
    }
}

fn nonempty_field(fields: &[&str], index: usize) -> Option<String> {
    fields
        .get(index)
        .map(|field| field.trim())
        .filter(|field| !field.is_empty())
        .map(ToOwned::to_owned)
}

fn numeric_field(fields: &[&str], index: usize) -> Option<u64> {
    fields
        .get(index)
        .map(|field| field.trim())
        .filter(|field| !field.is_empty())
        .and_then(|field| field.parse().ok())
}

fn capture_packets(
    application: &str,
    root_pid: u32,
    duration_seconds: u64,
    output: &Path,
    include_children: bool,
) -> Result<()> {
    let started_at_ms = timestamp_ms()?;
    let output_parent = output.parent().unwrap_or_else(|| Path::new("."));
    let capture_directory = Builder::new()
        .prefix(".omega-sniffer-")
        .tempdir_in(output_parent)
        .with_context(|| {
            format!(
                "failed to create a protected temporary capture directory in {}",
                output_parent.display()
            )
        })?;
    let raw_path = capture_directory.path().join("unfiltered.pcapng");
    let stderr_path = capture_directory.path().join("tcpdump.stderr");
    let stderr_file = File::create(&stderr_path)
        .with_context(|| format!("failed to create {}", stderr_path.display()))?;
    let mut tcpdump = Command::new("/usr/sbin/tcpdump")
        .args(["-i", "pktap,all", "--apple-pcapng", "-s", "0", "-w"])
        .arg(&raw_path)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .context("failed to start tcpdump")?;

    thread::sleep(Duration::from_millis(250));
    if let Some(exit_status) = tcpdump.try_wait().context("failed to query tcpdump")? {
        let stderr = fs::read_to_string(&stderr_path)
            .with_context(|| format!("failed to read {}", stderr_path.display()))?;
        bail!(
            "packet capture could not start ({exit_status}): {}\nRun omega-sniffer with packet-capture privileges, for example: sudo omega-sniffer capture ...",
            stderr.trim()
        );
    }

    let deadline = Instant::now() + Duration::from_secs(duration_seconds);
    let mut observed_pids = BTreeSet::new();
    while Instant::now() < deadline {
        observed_pids.extend(
            related_processes(root_pid, include_children)?
                .keys()
                .copied(),
        );
        thread::sleep(Duration::from_millis(100));
    }
    stop_capture(&mut tcpdump)?;

    let filter = observed_pids
        .iter()
        .flat_map(|pid| [format!("pid={pid}"), format!("epid={pid}")])
        .collect::<Vec<_>>()
        .join(" || ");
    let filter_output = command_output(
        Command::new("/usr/sbin/tcpdump")
            .args(["-r"])
            .arg(&raw_path)
            .args(["-w"])
            .arg(output)
            .args(["-Q", &filter]),
        "filtering packet capture by process",
    )?;
    if !filter_output.status.success() {
        bail!(
            "failed to filter packet capture: {}",
            String::from_utf8_lossy(&filter_output.stderr).trim()
        );
    }
    let manifest = CaptureManifest {
        application: application.to_string(),
        root_pid,
        observed_pids: observed_pids.into_iter().collect(),
        started_at_ms,
        duration_seconds,
        format: "pcapng-packet-bytes",
        output: output.to_path_buf(),
    };
    write_manifest(output, &manifest)?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

fn stop_capture(child: &mut Child) -> Result<()> {
    let pid = i32::try_from(child.id()).context("tcpdump PID exceeded i32")?;
    // SIGINT asks tcpdump to flush its pcapng buffers and write a valid trailer.
    let result = unsafe { libc::kill(pid, libc::SIGINT) };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("failed to stop tcpdump");
    }
    let status = child.wait().context("failed to wait for tcpdump")?;
    if !status.success() {
        bail!("tcpdump exited with {status}");
    }
    Ok(())
}

fn write_manifest(output: &Path, manifest: &CaptureManifest) -> Result<()> {
    let manifest_path = output.with_extension(format!(
        "{}.manifest.json",
        output
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("capture")
    ));
    fs::write(&manifest_path, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn inspect(input: &Path, limit: usize) -> Result<()> {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some("jsonl") => inspect_jsonl(input),
        Some("pcap") | Some("pcapng") => inspect_pcap(input, limit),
        _ => bail!("unsupported artifact extension; expected .jsonl, .pcap, or .pcapng"),
    }
}

fn inspect_jsonl(input: &Path) -> Result<()> {
    let file = File::open(input).with_context(|| format!("failed to open {}", input.display()))?;
    let mut sample_count = 0;
    let mut first_timestamp_ms = None;
    let mut last_timestamp_ms = None;
    let mut processes: BTreeMap<(String, u32), ProcessSummary> = BTreeMap::new();

    for line in BufReader::new(file).lines() {
        let sample: FlowSample =
            serde_json::from_str(&line.context("failed to read JSONL artifact")?)
                .context("failed to parse JSONL artifact")?;
        sample_count += 1;
        first_timestamp_ms.get_or_insert(sample.timestamp_ms);
        last_timestamp_ms = Some(sample.timestamp_ms);
        let summary = processes
            .entry((sample.process.clone(), sample.pid))
            .or_insert_with(|| ProcessSummary {
                process: sample.process,
                pid: sample.pid,
                flows: Vec::new(),
                maximum_bytes_in: 0,
                maximum_bytes_out: 0,
            });
        if let Some(flow) = sample.flow
            && !summary.flows.contains(&flow)
        {
            summary.flows.push(flow);
        }
        summary.maximum_bytes_in = summary
            .maximum_bytes_in
            .max(sample.bytes_in.unwrap_or_default());
        summary.maximum_bytes_out = summary
            .maximum_bytes_out
            .max(sample.bytes_out.unwrap_or_default());
    }

    let summary = JsonlSummary {
        format: "jsonl-flow-metadata",
        samples: sample_count,
        processes: processes.into_values().collect(),
        first_timestamp_ms,
        last_timestamp_ms,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn inspect_pcap(input: &Path, limit: usize) -> Result<()> {
    let count_output = command_output(
        Command::new("/usr/sbin/tcpdump")
            .args(["--count", "-r"])
            .arg(input),
        "counting captured packets",
    )?;
    if !count_output.status.success() {
        bail!(
            "tcpdump could not count the artifact: {}",
            String::from_utf8_lossy(&count_output.stderr).trim()
        );
    }
    let packet_count = String::from_utf8_lossy(&count_output.stdout)
        .trim()
        .parse()
        .context("tcpdump returned an invalid packet count")?;
    let output = command_output(
        Command::new("/usr/sbin/tcpdump")
            .args(["-nn", "-XX", "-r"])
            .arg(input)
            .args(["-k", "INPDS", "-c", &limit.to_string()]),
        "decoding packet capture",
    )?;
    if !output.status.success() {
        bail!(
            "tcpdump could not decode the artifact: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let decoded_lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let summary = PcapSummary {
        format: "pcapng-packet-bytes",
        packet_count,
        decoded_lines,
    };
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn command_output(command: &mut Command, operation: &str) -> Result<Output> {
    command
        .output()
        .with_context(|| format!("failed while {operation}"))
}

fn timestamp_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nettop_processes_and_flows() {
        let processes = BTreeMap::from([(705, "/Applications/iTerm.app/iTerm2".to_string())]);
        let samples = parse_nettop(
            "time,,interface,state,bytes_in,bytes_out,\niTerm2.705,,,12,34,\ntcp4 127.0.0.1:1<->127.0.0.1:2,lo0,Established,10,20,\n",
            705,
            &processes,
        )
        .expect("sample parses");
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].pid, 705);
        assert_eq!(samples[0].bytes_out, Some(34));
        assert_eq!(
            samples[1].flow.as_deref(),
            Some("tcp4 127.0.0.1:1<->127.0.0.1:2")
        );
        assert_eq!(samples[1].interface.as_deref(), Some("lo0"));
    }

    #[test]
    fn ignores_unrelated_nettop_processes() {
        let processes = BTreeMap::from([(705, "iTerm2".to_string())]);
        let samples = parse_nettop(
            "other.12,,,100,200,\ntcp4 1.1.1.1:1<->2.2.2.2:2,en0,Established,100,200,\n",
            705,
            &processes,
        )
        .expect("sample parses");
        assert!(samples.is_empty());
    }
}
