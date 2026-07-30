# Alpha Feedback

Omega is in alpha. Bugs and rough edges are expected, and reports from
installed candidates are the main signal for what gets fixed next. This page is
the operational contract for that feedback: where to report, what to include,
who triages it, and what response to expect.

## Where to report

| What you have                                            | Where it goes                                                                                              |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| A reproducible defect                                    | The [bug report form](https://github.com/OpenAgentsInc/omega/issues/new?template=10_bug_report.yml)        |
| A crash, freeze, or hang                                 | The [crash report form](https://github.com/OpenAgentsInc/omega/issues/new?template=11_crash_report.yml)    |
| Anything loose — impressions, friction, questions, ideas | The alpha feedback tester channel in the sidebar (the channels section, `#alpha-feedback` where available) |

Loose reports stay in the tester channel. If a channel report turns out to be a
reproducible defect, a triager will ask you to file it through the bug form so
it binds to a build and gets a severity. If the channel relay is unreachable,
fall back to the GitHub forms above — they always work.

## Bind every report to a build

Every report must name the exact build it was observed on. A report that
cannot be bound to a candidate cannot be triaged.

- The fastest source is the **sidebar footer**: the Settings row shows the
  running version plus its build number, for example `v0.2.0 b28` on an
  installed candidate or `v0.2.0 dev` on a source build. Paste that string
  verbatim. The build number is the candidate's RC number, so `v0.2.0 b28`
  names release `v0.2.0-rc28` exactly.
- For GitHub reports, run `omega: copy system specs into clipboard` from the
  command palette. It captures the version, full commit, release channel, OS,
  architecture, and memory in one paste — the footer deliberately omits the
  commit sha, so use system specs when triage needs it.
- Also name the platform (for example macOS arm64) and the mode or surface you
  were in (which conversation mode, panel, or flow) when the defect appeared.

## Triage ownership

**Owner lane.** OpenAgents maintainers own triage. Every new issue filed
through the forms starts labelled `state:needs triage`. Triage assigns one
severity label (`severity:s0` through `severity:s3`), removes
`state:needs triage`, and posts the first response inside the window for that
severity. Closing an issue always carries a stated reason — no silent closes.

**Agent lane.** Crash intake also flows through Sentry. Triage agents fetch
crash reports with `script/sentry-fetch`, then follow the crash prompts in
`.factory/prompts/crash/` (`investigate.md` → `fix.md` → `link-issues.md`) to
reproduce, root-cause, link duplicate issues, and propose fixes. Agents may
label and comment; a maintainer confirms severity and closure.

## Severity ladder

| Severity      | Definition                                                                       | Response expectation                                                           |
| ------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `severity:s0` | Data loss or corruption, crash on launch, security or privacy exposure           | Acknowledged within 24 hours; fix or mitigation targeted at the next candidate |
| `severity:s1` | Crash, hang, or broken core flow (open, edit, converse, send) with no workaround | Acknowledged within 2 days; targeted at the next candidate                     |
| `severity:s2` | Wrong behavior with a workaround, or a defect outside the core flows             | Acknowledged within 7 days; scheduled on the roadmap                           |
| `severity:s3` | Polish — visual rough edges, copy, minor friction                                | Batched and tracked; no individual response promised                           |

Channel reports get a reply (or a redirect to the bug form) within 2 days.

## Privacy and logs

### What the Omega log contains

The app log lives at `~/Library/Logs/omega-rc/omega-rc.log` on an installed
macOS candidate (source builds use `omega-dev`; on Linux the logs directory
sits under the app data directory). Expect it to contain:

- timestamps and module-level diagnostics
- error and refusal lines — typed refusals name the policy or precondition
  that refused an action
- conversation **thread titles**
- workspace **file paths** and language server names
- relay URLs and connection state

The log is not supposed to contain message bodies or secrets, but treat it as
sensitive and review it before sharing.

### Safe log attachment path

1. Run `omega: open log` from the command palette (last 1000 lines), or
   `omega: reveal log in file manager` for the full file.
2. Trim to the window around the defect.
3. Redact anything private: credentials of any kind, thread titles you do not
   want public, file paths that reveal private project names.
4. Paste the result inside the collapsed log section of the bug or crash form.

### Never paste

- API keys, tokens, or credentials of any kind (`sk-…`, bearer tokens,
  `nsec…` Nostr secret keys, session cookies)
- full conversation transcripts that include private code
- private repository contents beyond the minimal reproduction
- other people's private messages

GitHub issues and the tester channels are public and permanent. macOS crash
files under `~/Library/Logs/DiagnosticReports` can embed file paths — review
them like logs before attaching.

## The round trip

A healthy report looks like this: a tester on an installed candidate files a
bug with the footer version string, reproduction steps, and a redacted log
excerpt; triage assigns a severity within the window above; the fix lands and
the issue is closed naming the candidate that carries it. If your report does
not get a response inside the published window, say so in the tester channel —
that is itself an S2 defect in this process.
