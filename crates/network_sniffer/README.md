# Omega network sniffer

`omega-sniffer` captures traffic from an already-running macOS application. The
application does not need to integrate with Omega. Select it by PID, application
name, or bundle identifier; child processes are included by default.

Full packet bytes, including Apple's PID and process metadata, require macOS
packet-capture privileges:

```sh
sudo omega-sniffer capture \
  --application com.googlecode.iterm2 \
  --duration 30 \
  --output /tmp/iterm.pcapng
```

For an unprivileged inventory of flows and byte counters:

```sh
omega-sniffer capture \
  --application com.googlecode.iterm2 \
  --duration 30 \
  --format jsonl \
  --output /tmp/iterm.jsonl
```

Both formats can be summarized as JSON for agent analysis:

```sh
omega-sniffer inspect --input /tmp/iterm.jsonl
omega-sniffer inspect --input /tmp/iterm.pcapng --limit 100
```

Pcapng contains captured wire bytes, but encrypted protocols such as TLS remain
encrypted. JSONL contains endpoints, interface, connection state, and byte
counters; it does not contain packet payloads.
