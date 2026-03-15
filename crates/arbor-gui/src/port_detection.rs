use super::*;

#[derive(Clone)]
pub(crate) struct PortScanTarget {
    pub(crate) worktree_path: PathBuf,
    pub(crate) root_pid: u32,
}

#[derive(Clone)]
pub(crate) struct ProcessInfoSnapshot {
    parent_pid: u32,
    #[cfg_attr(unix, allow(dead_code))]
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScannedPortInfo {
    port: u16,
    pid: u32,
    address: String,
    process_name: String,
}

const IGNORED_PORTS: [u16; 7] = [22, 80, 443, 3306, 5432, 6379, 27017];

pub(crate) fn detect_ports_for_worktrees(
    worktree_paths: &[PathBuf],
    scan_targets: &[PortScanTarget],
    terminal_output_hints: &HashMap<PathBuf, String>,
) -> HashMap<PathBuf, Vec<DetectedPort>> {
    let mut ports_by_worktree: HashMap<PathBuf, Vec<DetectedPort>> = HashMap::new();
    let worktrees_with_pid_targets: HashSet<PathBuf> = scan_targets
        .iter()
        .map(|target| target.worktree_path.clone())
        .collect();
    let mut dynamic_paths = Vec::new();

    for worktree_path in worktree_paths {
        match load_static_ports_for_worktree(worktree_path) {
            Ok(Some(static_ports)) => {
                ports_by_worktree.insert(worktree_path.clone(), static_ports);
            },
            Ok(None) => dynamic_paths.push(worktree_path.clone()),
            Err(error) => {
                tracing::warn!(path = %worktree_path.display(), %error, "invalid static port config");
                ports_by_worktree.insert(worktree_path.clone(), Vec::new());
            },
        }
    }

    let process_snapshot = list_process_snapshot();
    let pid_owner_map = build_pid_owner_map(scan_targets, &process_snapshot);
    let scanned_ports = list_listening_ports_for_pids(
        &pid_owner_map.keys().copied().collect::<Vec<_>>(),
        &process_snapshot,
    );
    for port_info in scanned_ports {
        if IGNORED_PORTS.contains(&port_info.port) {
            continue;
        }
        let Some(worktree_path) = pid_owner_map.get(&port_info.pid) else {
            continue;
        };
        ports_by_worktree
            .entry(worktree_path.clone())
            .or_default()
            .push(DetectedPort {
                port: port_info.port,
                pid: Some(port_info.pid),
                address: port_info.address,
                process_name: port_info.process_name,
                label: None,
            });
    }

    for worktree_path in dynamic_paths {
        let current_ports = ports_by_worktree.entry(worktree_path.clone()).or_default();
        if current_ports.is_empty()
            && !worktrees_with_pid_targets.contains(&worktree_path)
            && let Some(output) = terminal_output_hints.get(&worktree_path)
        {
            current_ports.extend(
                extract_ports_from_terminal_output(output)
                    .into_iter()
                    .filter(|port| !IGNORED_PORTS.contains(&port.port)),
            );
        }
    }

    for ports in ports_by_worktree.values_mut() {
        ports.sort_by(|left, right| {
            left.port
                .cmp(&right.port)
                .then(left.address.cmp(&right.address))
                .then(left.label.cmp(&right.label))
        });
        ports.dedup_by(|left, right| {
            left.port == right.port && left.address == right.address && left.label == right.label
        });
    }

    ports_by_worktree.retain(|_, ports| !ports.is_empty());
    ports_by_worktree
}

fn load_static_ports_for_worktree(
    worktree_path: &Path,
) -> Result<Option<Vec<DetectedPort>>, StoreError> {
    let path = worktree_path.join(".arbor").join("ports.json");
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path).map_err(|source| StoreError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let config = serde_json::from_str::<StaticPortsConfig>(&content).map_err(|source| {
        StoreError::JsonParse {
            path: path.display().to_string(),
            source,
        }
    })?;

    Ok(Some(
        config
            .ports
            .into_iter()
            .filter(|entry| entry.port > 0)
            .map(|entry| DetectedPort {
                port: entry.port,
                pid: None,
                address: "127.0.0.1".to_owned(),
                process_name: "configured".to_owned(),
                label: entry.label.and_then(|label| {
                    let trimmed = label.trim().to_owned();
                    (!trimmed.is_empty()).then_some(trimmed)
                }),
            })
            .collect(),
    ))
}

#[derive(serde::Deserialize)]
struct StaticPortsConfig {
    #[serde(default)]
    ports: Vec<StaticPortEntry>,
}

#[derive(serde::Deserialize)]
struct StaticPortEntry {
    port: u16,
    label: Option<String>,
}

fn build_pid_owner_map(
    scan_targets: &[PortScanTarget],
    process_snapshot: &HashMap<u32, ProcessInfoSnapshot>,
) -> HashMap<u32, PathBuf> {
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, process) in process_snapshot {
        children_by_parent
            .entry(process.parent_pid)
            .or_default()
            .push(pid);
    }

    let mut pid_owner_map = HashMap::new();
    for target in scan_targets {
        let mut stack = vec![target.root_pid];
        while let Some(pid) = stack.pop() {
            if pid_owner_map.contains_key(&pid) {
                continue;
            }
            pid_owner_map.insert(pid, target.worktree_path.clone());
            if let Some(children) = children_by_parent.get(&pid) {
                stack.extend(children.iter().copied());
            }
        }
    }

    pid_owner_map
}

#[cfg(unix)]
fn list_process_snapshot() -> HashMap<u32, ProcessInfoSnapshot> {
    let mut command = create_command("ps");
    command.args(["-axo", "pid=,ppid=,comm="]);
    let output = match run_command_output(&mut command, "list processes") {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    parse_unix_process_snapshot(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(windows)]
fn list_process_snapshot() -> HashMap<u32, ProcessInfoSnapshot> {
    let mut command = create_command("powershell");
    command.args([
        "-NoProfile",
        "-Command",
        "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress",
    ]);
    let output = match run_command_output(&mut command, "list processes") {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    parse_windows_process_snapshot(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(any(unix, windows)))]
fn list_process_snapshot() -> HashMap<u32, ProcessInfoSnapshot> {
    HashMap::new()
}

#[cfg(unix)]
fn list_listening_ports_for_pids(
    pids: &[u32],
    _process_snapshot: &HashMap<u32, ProcessInfoSnapshot>,
) -> Vec<ScannedPortInfo> {
    if pids.is_empty() {
        return Vec::new();
    }

    let pid_arg = pids
        .iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    let mut command = create_command("sh");
    command.arg("-lc").arg(format!(
        "lsof -p {pid_arg} -iTCP -sTCP:LISTEN -P -n 2>/dev/null || true"
    ));
    let output = match run_command_output(&mut command, "scan listening ports") {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    parse_unix_lsof_ports(&String::from_utf8_lossy(&output.stdout), &pid_set)
}

#[cfg(windows)]
fn list_listening_ports_for_pids(
    pids: &[u32],
    process_snapshot: &HashMap<u32, ProcessInfoSnapshot>,
) -> Vec<ScannedPortInfo> {
    if pids.is_empty() {
        return Vec::new();
    }

    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    let mut command = create_command("netstat");
    command.args(["-ano", "-p", "tcp"]);
    let output = match run_command_output(&mut command, "scan listening ports") {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    parse_windows_netstat_ports(
        &String::from_utf8_lossy(&output.stdout),
        &pid_set,
        process_snapshot,
    )
}

#[cfg(not(any(unix, windows)))]
fn list_listening_ports_for_pids(
    _pids: &[u32],
    _process_snapshot: &HashMap<u32, ProcessInfoSnapshot>,
) -> Vec<ScannedPortInfo> {
    Vec::new()
}

#[cfg(unix)]
fn parse_unix_process_snapshot(output: &str) -> HashMap<u32, ProcessInfoSnapshot> {
    let mut processes = HashMap::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Some(parent_pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let name = parts.collect::<Vec<_>>().join(" ");
        processes.insert(pid, ProcessInfoSnapshot { parent_pid, name });
    }
    processes
}

#[cfg(windows)]
fn parse_windows_process_snapshot(output: &str) -> HashMap<u32, ProcessInfoSnapshot> {
    let parsed = match serde_json::from_str::<serde_json::Value>(output.trim()) {
        Ok(parsed) => parsed,
        Err(_) => return HashMap::new(),
    };
    let entries = match parsed {
        serde_json::Value::Array(values) => values,
        value => vec![value],
    };

    let mut processes = HashMap::new();
    for entry in entries {
        let Some(pid) = entry.get("ProcessId").and_then(|value| value.as_u64()) else {
            continue;
        };
        let Some(parent_pid) = entry
            .get("ParentProcessId")
            .and_then(|value| value.as_u64())
        else {
            continue;
        };
        let name = entry
            .get("Name")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_owned();
        processes.insert(pid as u32, ProcessInfoSnapshot {
            parent_pid: parent_pid as u32,
            name,
        });
    }
    processes
}

#[cfg(unix)]
fn parse_unix_lsof_ports(output: &str, pid_set: &HashSet<u32>) -> Vec<ScannedPortInfo> {
    let mut ports = Vec::new();
    for line in output.lines().skip(1) {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 10 {
            continue;
        }
        let Some(pid) = columns.get(1).and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if !pid_set.contains(&pid) {
            continue;
        }
        let Some(name_field) = columns.get(columns.len().saturating_sub(2)).copied() else {
            continue;
        };
        let Some((address, port)) = parse_socket_address_port(name_field) else {
            continue;
        };
        ports.push(ScannedPortInfo {
            port,
            pid,
            address,
            process_name: columns[0].to_owned(),
        });
    }
    ports
}

#[cfg(windows)]
fn parse_windows_netstat_ports(
    output: &str,
    pid_set: &HashSet<u32>,
    process_snapshot: &HashMap<u32, ProcessInfoSnapshot>,
) -> Vec<ScannedPortInfo> {
    let mut ports = Vec::new();
    for line in output.lines() {
        if !line.contains("LISTENING") {
            continue;
        }
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 5 {
            continue;
        }
        let Some(pid) = columns.last().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if !pid_set.contains(&pid) {
            continue;
        }
        let Some((address, port)) = parse_socket_address_port(columns[1]) else {
            continue;
        };
        let process_name = process_snapshot
            .get(&pid)
            .map(|process| process.name.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        ports.push(ScannedPortInfo {
            port,
            pid,
            address,
            process_name,
        });
    }
    ports
}

fn parse_socket_address_port(value: &str) -> Option<(String, u16)> {
    if let Some((address, port_text)) = value.rsplit_once(':')
        && let Ok(port) = port_text.parse::<u16>()
    {
        let normalized = address
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let normalized = if normalized == "*" {
            "0.0.0.0".to_owned()
        } else {
            normalized
        };
        return Some((normalized, port));
    }
    None
}

pub(crate) fn extract_ports_from_terminal_output(output: &str) -> Vec<DetectedPort> {
    const ADDRESS_MARKERS: [(&str, &str); 10] = [
        ("http://127.0.0.1:", "127.0.0.1"),
        ("https://127.0.0.1:", "127.0.0.1"),
        ("http://localhost:", "127.0.0.1"),
        ("https://localhost:", "127.0.0.1"),
        ("http://0.0.0.0:", "0.0.0.0"),
        ("https://0.0.0.0:", "0.0.0.0"),
        ("127.0.0.1:", "127.0.0.1"),
        ("localhost:", "127.0.0.1"),
        ("0.0.0.0:", "0.0.0.0"),
        ("[::]:", "::"),
    ];
    const PHRASE_MARKERS: [(&str, &str); 5] = [
        ("listening on port ", "127.0.0.1"),
        ("listening at port ", "127.0.0.1"),
        ("running on port ", "127.0.0.1"),
        ("ready on port ", "127.0.0.1"),
        ("server started on port ", "127.0.0.1"),
    ];

    let mut ports = Vec::new();
    for (marker, address) in ADDRESS_MARKERS {
        collect_port_markers(&mut ports, output, marker, address);
    }

    let lowercase = output.to_ascii_lowercase();
    for (marker, address) in PHRASE_MARKERS {
        collect_port_markers(&mut ports, &lowercase, marker, address);
    }

    ports.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then(left.address.cmp(&right.address))
            .then(left.label.cmp(&right.label))
    });
    ports.dedup_by(|left, right| {
        left.port == right.port && left.address == right.address && left.label == right.label
    });
    ports
}

fn collect_port_markers(
    ports: &mut Vec<DetectedPort>,
    haystack: &str,
    marker: &str,
    address: &str,
) {
    let mut remainder = haystack;
    while let Some(index) = remainder.find(marker) {
        let after_marker = &remainder[index + marker.len()..];
        let digits: String = after_marker
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        if let Ok(port) = digits.parse::<u16>() {
            ports.push(DetectedPort {
                port,
                pid: None,
                address: address.to_owned(),
                process_name: "hint".to_owned(),
                label: None,
            });
        }
        remainder = after_marker;
    }
}

pub(crate) fn output_contains_port_hint(output: &str) -> bool {
    if !extract_ports_from_terminal_output(output).is_empty() {
        return true;
    }

    let lowercase = output.to_ascii_lowercase();
    [
        "listening on port",
        "listening at port",
        "server started on",
        "server running on",
        "ready on",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
}

pub(crate) fn worktree_port_url(port: &DetectedPort) -> String {
    let host = match port.address.as_str() {
        "" | "*" | "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    };
    format!("http://{host}:{}", port.port)
}

pub(crate) fn worktree_port_badge_text(port: &DetectedPort) -> String {
    format!(":{}", port.port)
}

pub(crate) fn worktree_port_detail_text(port: &DetectedPort) -> String {
    if let Some(label) = port
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        return format!("{label} :{}", port.port);
    }
    if port.process_name != "hint"
        && port.process_name != "configured"
        && !port.process_name.trim().is_empty()
    {
        return format!("{} :{}", port.process_name, port.port);
    }
    format!(":{}", port.port)
}

impl ArborWindow {
    pub(crate) fn refresh_worktree_ports(&mut self, cx: &mut Context<Self>) {
        let worktree_paths: Vec<PathBuf> = self
            .worktrees
            .iter()
            .map(|worktree| worktree.path.clone())
            .collect();
        if worktree_paths.is_empty() {
            return;
        }

        let scan_targets: Vec<PortScanTarget> = self
            .terminals
            .iter()
            .filter(|session| {
                session.state == TerminalState::Running
                    && session
                        .runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.kind() == TerminalRuntimeKind::Local)
            })
            .filter_map(|session| {
                session.root_pid.map(|root_pid| PortScanTarget {
                    worktree_path: session.worktree_path.clone(),
                    root_pid,
                })
            })
            .collect();
        let terminal_output_hints: HashMap<PathBuf, String> = worktree_paths
            .iter()
            .map(|worktree_path| {
                let mut combined = String::new();
                for session in self
                    .terminals
                    .iter()
                    .filter(|session| session.worktree_path == *worktree_path)
                {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&terminal_output_tail_for_metadata(session, 48, 8_000));
                }
                (worktree_path.clone(), combined)
            })
            .collect();

        cx.spawn(async move |this, cx| {
            let detected = cx
                .background_spawn(async move {
                    detect_ports_for_worktrees(
                        &worktree_paths,
                        &scan_targets,
                        &terminal_output_hints,
                    )
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for worktree in &mut this.worktrees {
                    let next_ports = detected.get(&worktree.path).cloned().unwrap_or_default();
                    if worktree.detected_ports != next_ports {
                        worktree.detected_ports = next_ports;
                        changed = true;
                    }
                }

                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn extract_ports_from_terminal_output_detects_common_local_urls() {
        let mut ports = extract_ports_from_terminal_output(
            "ready on http://localhost:3000 and http://127.0.0.1:5173",
        );
        ports.sort_by_key(|port| port.port);
        ports.dedup_by_key(|port| port.port);

        assert_eq!(
            ports.into_iter().map(|port| port.port).collect::<Vec<_>>(),
            vec![3000, 5173]
        );
    }

    #[test]
    fn output_contains_port_hint_detects_phrase_without_url() {
        assert!(output_contains_port_hint(
            "Server started on port 4173 in 220ms"
        ));
    }
}
