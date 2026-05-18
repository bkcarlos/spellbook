use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct Inference {
    pub title: String,
    pub command: String,
    pub category: String,
    pub tags: Vec<String>,
}

pub fn infer(raw: &str) -> Inference {
    let (title, command) = split_title_and_command(raw);
    let tokens = tokenize(&command);
    let category = infer_category(&tokens);
    let tags = infer_tags(&tokens, &command);
    Inference {
        title,
        command,
        category,
        tags,
    }
}

/// Split a raw paste into (title, command).
/// Rules:
/// 1. If a leading `#` comment line exists, use it as title; the rest is command.
/// 2. Otherwise, use the first non-empty line of the command (truncated to 60 chars) as the title.
fn split_title_and_command(raw: &str) -> (String, String) {
    let trimmed = raw.trim_matches(|c: char| c == '\n' || c == '\r');
    let lines: Vec<&str> = trimmed.lines().collect();

    if let Some(first) = lines.iter().find(|l| !l.trim().is_empty()) {
        let t = first.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let title = rest.trim_start_matches('!').trim().to_string();
            let command: String = lines
                .iter()
                .skip_while(|l| l.trim() != *first)
                .skip(1)
                .copied()
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            return (title, command);
        }
    }

    let command = strip_prompt(trimmed).to_string();
    let title_src = command.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let title = truncate(title_src.trim(), 60);
    (title, command)
}

fn strip_prompt(s: &str) -> String {
    // strip leading `$ ` or `> ` shell prompts from each line
    s.lines()
        .map(|l| {
            let t = l.trim_start();
            if let Some(r) = t.strip_prefix("$ ") {
                r.to_string()
            } else if let Some(r) = t.strip_prefix("> ") {
                r.to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(n - 1).collect();
        format!("{}…", truncated)
    }
}

fn tokenize(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn infer_category(tokens: &[String]) -> String {
    let first = tokens.first().map(|s| s.as_str()).unwrap_or("");
    let second = tokens.get(1).map(|s| s.as_str()).unwrap_or("");
    let key = match (first, second) {
        ("docker", "compose") => "docker-compose",
        ("git", "lfs") => "git",
        _ => first,
    };

    let cat = match key {
        // Git
        "git" | "gh" | "hub" => "Git",
        // Docker
        "docker" | "docker-compose" | "podman" | "buildah" | "skopeo" => "Docker",
        // Kubernetes
        "kubectl" | "helm" | "k9s" | "kustomize" | "kubeadm" | "minikube" | "kind" => "Kubernetes",
        // SSH
        "ssh" | "scp" | "rsync" | "ssh-keygen" | "ssh-copy-id" | "sshfs" | "ssh-add" => "SSH",
        // Database
        "mysql" | "psql" | "mongo" | "mongosh" | "redis-cli" | "sqlite3" | "mongodump"
        | "mongorestore" | "pg_dump" | "pg_restore" | "mysqldump" => "Database",
        // NodeJS
        "npm" | "yarn" | "pnpm" | "node" | "npx" | "bun" => "NodeJS",
        // Python
        "python" | "python3" | "pip" | "pip3" | "poetry" | "uv" | "conda" | "pipx" => "Python",
        // Rust
        "cargo" | "rustc" | "rustup" | "rustfmt" | "clippy-driver" => "Rust",
        // Go
        "go" | "gofmt" | "goimports" => "Go",
        // Media
        "ffmpeg" | "ffprobe" | "convert" | "magick" | "imagemagick" | "yt-dlp" => "Media",
        // HTTP
        "curl" | "wget" | "http" | "httpie" | "xh" => "HTTP",
        // System
        "systemctl" | "journalctl" | "service" | "launchctl" | "ps" | "top" | "htop"
        | "lsof" | "netstat" | "ss" | "killall" | "kill" | "uptime" | "df" | "du"
        | "free" | "iostat" | "vmstat" => "System",
        // Package
        "brew" | "apt" | "apt-get" | "yum" | "dnf" | "pacman" | "snap" | "flatpak" | "port" => "Package",
        // Linux core utils
        "ls" | "cd" | "cp" | "mv" | "rm" | "mkdir" | "rmdir" | "touch" | "find" | "grep"
        | "sed" | "awk" | "tar" | "zip" | "unzip" | "gzip" | "gunzip" | "chmod" | "chown"
        | "ln" | "head" | "tail" | "less" | "more" | "cat" | "echo" | "tr" | "cut"
        | "sort" | "uniq" | "wc" | "xargs" | "tee" | "diff" | "patch" | "stat" | "file"
        | "which" | "whereis" | "type" | "alias" | "export" | "env" => "Linux",
        _ => "Other",
    };

    cat.to_string()
}

fn infer_tags(tokens: &[String], command: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let push = |t: &str, tags: &mut Vec<String>, seen: &mut HashSet<String>| {
        let t = t.to_lowercase();
        if !t.is_empty() && seen.insert(t.clone()) {
            tags.push(t);
        }
    };

    // Rule 1: tool name
    if let Some(first) = tokens.first() {
        if KNOWN_TOOLS.contains(&first.as_str()) {
            push(first, &mut tags, &mut seen);
        }
    }

    // Rule 3: subcommand (tool + subcommand)
    if tokens.len() >= 2 {
        let combo = format!("{} {}", tokens[0], tokens[1]);
        if let Some(sub) = SUBCOMMAND_TAGS.iter().find(|(c, _)| *c == combo) {
            push(sub.1, &mut tags, &mut seen);
        }
    }

    // Rule 2: action verbs
    for tok in tokens.iter().take(6) {
        if let Some(action) = VERB_TAGS.iter().find(|(k, _)| *k == tok.as_str()) {
            push(action.1, &mut tags, &mut seen);
        }
    }

    // Rule 4: port / protocol
    if command.contains("://") {
        if command.contains("https://") {
            push("https", &mut tags, &mut seen);
        } else if command.contains("http://") {
            push("http", &mut tags, &mut seen);
        }
    }
    if has_port(command) {
        push("port", &mut tags, &mut seen);
    }

    tags.truncate(5);
    tags
}

/// Detect `:NNNN` (a port-like sequence) anywhere in the command.
/// Heuristic: a `:` whose preceding byte is NOT an ASCII digit, followed
/// by 2-5 digits. This rejects time-of-day like `12:34:56` (the second `:`
/// is preceded by `4`) and lets through `host:22`, `:443`, `localhost:8080`.
fn has_port(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && (i == 0 || !bytes[i - 1].is_ascii_digit()) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let digits = j - i - 1;
            if (2..=5).contains(&digits) {
                return true;
            }
        }
        i += 1;
    }
    false
}

const KNOWN_TOOLS: &[&str] = &[
    "docker", "git", "kubectl", "helm", "npm", "yarn", "pnpm", "cargo", "rustc", "python",
    "python3", "pip", "node", "go", "mvn", "gradle", "ssh", "scp", "rsync", "curl", "wget",
    "ffmpeg", "tar", "zip", "unzip", "grep", "sed", "awk", "find", "lsof", "netstat",
    "systemctl", "journalctl", "brew", "apt", "yum", "pacman", "nginx", "redis-cli", "mysql",
    "psql", "mongo", "mongosh", "kubectl", "podman", "make", "cmake", "openssl", "ssh-keygen",
    "ssh-copy-id", "ssh-add", "bun", "deno", "poetry", "uv", "conda", "rustup", "gofmt",
    "yt-dlp", "ps", "top", "htop", "kill", "killall", "tmux", "screen", "vim", "nvim", "git-lfs",
    "docker-compose",
];

const VERB_TAGS: &[(&str, &str)] = &[
    ("rm", "cleanup"),
    ("prune", "cleanup"),
    ("delete", "cleanup"),
    ("remove", "cleanup"),
    ("clean", "cleanup"),
    ("ls", "list"),
    ("list", "list"),
    ("ps", "list"),
    ("show", "list"),
    ("get", "list"),
    ("describe", "list"),
    ("restart", "restart"),
    ("reload", "restart"),
    ("log", "logs"),
    ("logs", "logs"),
    ("tail", "logs"),
    ("backup", "backup"),
    ("dump", "backup"),
    ("export", "backup"),
    ("install", "install"),
    ("add", "install"),
    ("kill", "stop"),
    ("stop", "stop"),
    ("down", "stop"),
    ("start", "start"),
    ("up", "start"),
    ("run", "start"),
    ("find", "search"),
    ("grep", "search"),
    ("search", "search"),
    ("watch", "monitor"),
    ("status", "status"),
    ("info", "status"),
    ("commit", "commit"),
    ("push", "remote"),
    ("pull", "remote"),
    ("fetch", "remote"),
    ("clone", "remote"),
    ("checkout", "branch"),
    ("merge", "merge"),
    ("rebase", "rebase"),
    ("stash", "stash"),
    ("diff", "diff"),
    ("apply", "apply"),
    ("exec", "exec"),
];

const SUBCOMMAND_TAGS: &[(&str, &str)] = &[
    ("docker compose", "compose"),
    ("docker container", "container"),
    ("docker image", "image"),
    ("docker network", "network"),
    ("docker volume", "volume"),
    ("docker buildx", "buildx"),
    ("git remote", "remote"),
    ("git stash", "stash"),
    ("git rebase", "rebase"),
    ("git submodule", "submodule"),
    ("kubectl get", "query"),
    ("kubectl apply", "apply"),
    ("kubectl describe", "describe"),
    ("kubectl exec", "exec"),
    ("kubectl logs", "logs"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_docker_prune() {
        let r = infer("docker container prune -f");
        assert_eq!(r.category, "Docker");
        assert!(r.tags.contains(&"docker".to_string()));
        assert!(r.tags.contains(&"container".to_string()));
        assert!(r.tags.contains(&"cleanup".to_string()));
    }

    #[test]
    fn uses_comment_as_title() {
        let r = infer("# Backup current dir\ntar -czf b.tgz .");
        assert_eq!(r.title, "Backup current dir");
        assert_eq!(r.command, "tar -czf b.tgz .");
        assert_eq!(r.category, "Linux");
    }

    #[test]
    fn strips_dollar_prompt() {
        let r = infer("$ ls -la");
        assert_eq!(r.command, "ls -la");
    }

    #[test]
    fn unknown_falls_to_other() {
        let r = infer("foobar --baz");
        assert_eq!(r.category, "Other");
    }

    #[test]
    fn detects_https() {
        let r = infer("curl https://example.com");
        assert!(r.tags.contains(&"https".to_string()));
        assert!(r.tags.contains(&"curl".to_string()));
    }

    #[test]
    fn empty_input_does_not_panic() {
        let r = infer("");
        assert_eq!(r.command, "");
        assert_eq!(r.title, "");
        assert_eq!(r.category, "Other");
        assert!(r.tags.is_empty());
    }

    #[test]
    fn whitespace_only_does_not_panic() {
        let r = infer("   \n  \t \n");
        assert_eq!(r.category, "Other");
    }

    #[test]
    fn detects_kubectl() {
        let r = infer("kubectl get pods -n default");
        assert_eq!(r.category, "Kubernetes");
        assert!(r.tags.contains(&"kubectl".to_string()));
        assert!(r.tags.contains(&"query".to_string()));
    }

    #[test]
    fn detects_git_subcommand() {
        let r = infer("git remote add origin git@github.com:foo/bar.git");
        assert_eq!(r.category, "Git");
        assert!(r.tags.contains(&"git".to_string()));
        assert!(r.tags.contains(&"remote".to_string()));
    }

    #[test]
    fn detects_docker_compose_combo() {
        let r = infer("docker compose up -d");
        assert_eq!(r.category, "Docker");
        assert!(r.tags.contains(&"compose".to_string()));
    }

    #[test]
    fn detects_port_in_url() {
        let r = infer("curl http://localhost:8080/health");
        assert!(r.tags.contains(&"port".to_string()));
        assert!(r.tags.contains(&"http".to_string()));
    }

    #[test]
    fn ssh_lands_in_ssh_category() {
        let r = infer("ssh-copy-id user@host");
        assert_eq!(r.category, "SSH");
    }

    #[test]
    fn database_clients_categorized() {
        for cmd in &["psql -h localhost", "redis-cli ping", "mysql -u root"] {
            let r = infer(cmd);
            assert_eq!(r.category, "Database", "{cmd}");
        }
    }

    #[test]
    fn multi_line_command_keeps_first_line_as_title() {
        let r = infer("tar -czf b.tgz a/\necho done");
        assert_eq!(r.title, "tar -czf b.tgz a/");
        assert!(r.command.contains("echo done"));
    }

    #[test]
    fn shebang_comment_used_as_title() {
        let r = infer("#!/bin/bash\nset -e");
        assert_eq!(r.title, "/bin/bash");
    }

    #[test]
    fn title_truncates_long_first_line() {
        let long = "a".repeat(120);
        let r = infer(&long);
        let chars = r.title.chars().count();
        assert!(chars <= 60, "title length should be <= 60 chars, got {chars}");
    }

    #[test]
    fn tags_capped_at_5() {
        // Saturate verb rules: tool + subcommand + many action verbs in first-6
        let r = infer("docker compose up list rm restart logs start");
        assert!(r.tags.len() <= 5, "got {} tags: {:?}", r.tags.len(), r.tags);
    }

    #[test]
    fn tags_are_lowercase_and_deduped() {
        let r = infer("git log");
        for t in &r.tags {
            assert_eq!(t, &t.to_lowercase());
        }
        let mut sorted = r.tags.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), r.tags.len(), "duplicate tag found in {:?}", r.tags);
    }

    #[test]
    fn strips_caret_prompt_too() {
        let r = infer("> ls -la");
        assert_eq!(r.command, "ls -la");
    }

    #[test]
    fn port_detector_handles_edge_cases() {
        assert!(has_port("curl http://localhost:8080"));
        assert!(has_port("user@host:22"));
        assert!(has_port(":443"));
        assert!(!has_port("12:34:56"), "time-of-day should not trigger");
        assert!(!has_port("no port here"));
        assert!(!has_port(":1"), "single digit too short for a port");
        assert!(!has_port(":1234567"), "too many digits for a port");
    }
}
