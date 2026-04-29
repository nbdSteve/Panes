use panes_events::RiskLevel;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static DESTRUCTIVE_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(rm\s+-rf|rm\s+-fr|rmdir|DROP\s+TABLE|DROP\s+DATABASE|TRUNCATE|DELETE\s+FROM|docker\s+rm|docker\s+rmi|docker\s+system\s+prune|git\s+push\s+--force|git\s+push\s+-f|git\s+reset\s+--hard|git\s+clean\s+-fd|FORMAT|mkfs|dd\s+if=|shutdown|reboot|kill\s+-9|pkill|killall)\b"
    ).unwrap()
});

static WRITE_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(rm\b|mv\b|cp\b|chmod\b|chown\b|git\s+push|git\s+commit|git\s+checkout|git\s+merge|git\s+rebase|npm\s+publish|pip\s+install|brew\s+install|apt\s+install|curl\s+.*-o|wget\b|mkdir\b|touch\b)\b"
    ).unwrap()
});

static READONLY_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(ls|cat|head|tail|wc|grep|find|which|pwd|echo|env|printenv|whoami|date|uname|hostname|git\s+(status|log|diff|show|branch|remote|rev-parse)|npm\s+(test|run\s+test|list|ls)|cargo\s+(test|check|clippy|build)|node\s+-[ev]|python\s+-c|ruby\s+-e)\b"
    ).unwrap()
});

pub fn classify_risk(tool_name: &str, input: &Value) -> RiskLevel {
    match tool_name {
        "Read" | "WebSearch" | "LSP" => RiskLevel::Low,

        "WebFetch" => RiskLevel::Low,

        "Write" | "NotebookEdit" => RiskLevel::Medium,

        "Edit" => RiskLevel::Medium,

        "Bash" => classify_bash_risk(input),

        "Task" | "Agent" => RiskLevel::Medium,

        _ => RiskLevel::Medium,
    }
}

fn classify_bash_risk(input: &Value) -> RiskLevel {
    let command = match input.get("command").and_then(|c| c.as_str()) {
        Some(cmd) => cmd,
        None => return RiskLevel::Medium,
    };

    if DESTRUCTIVE_PATTERNS.is_match(command) {
        return RiskLevel::Critical;
    }

    if READONLY_PATTERNS.is_match(command) {
        return RiskLevel::Low;
    }

    if WRITE_PATTERNS.is_match(command) {
        return RiskLevel::High;
    }

    RiskLevel::Medium
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_read_tools_are_low() {
        assert_eq!(classify_risk("Read", &json!({})), RiskLevel::Low);
        assert_eq!(classify_risk("WebSearch", &json!({})), RiskLevel::Low);
    }

    #[test]
    fn test_write_edit_are_medium() {
        assert_eq!(classify_risk("Write", &json!({})), RiskLevel::Medium);
        assert_eq!(classify_risk("Edit", &json!({})), RiskLevel::Medium);
    }

    #[test]
    fn test_bash_readonly_is_low() {
        let input = json!({"command": "ls -la"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Low);

        let input = json!({"command": "git status"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Low);

        let input = json!({"command": "npm test"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Low);

        let input = json!({"command": "cargo check"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Low);
    }

    #[test]
    fn test_bash_destructive_is_critical() {
        let input = json!({"command": "rm -rf /tmp/data"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Critical);

        let input = json!({"command": "DROP TABLE users"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Critical);

        let input = json!({"command": "git push --force origin main"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Critical);

        let input = json!({"command": "git reset --hard HEAD~5"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Critical);

        let input = json!({"command": "docker system prune -a"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Critical);
    }

    #[test]
    fn test_bash_write_is_high() {
        let input = json!({"command": "rm file.txt"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::High);

        let input = json!({"command": "git push origin main"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::High);

        let input = json!({"command": "chmod 755 script.sh"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::High);
    }

    #[test]
    fn test_bash_unknown_is_medium() {
        let input = json!({"command": "some-custom-tool --flag"});
        assert_eq!(classify_risk("Bash", &input), RiskLevel::Medium);
    }

    #[test]
    fn test_unknown_tool_is_medium() {
        assert_eq!(classify_risk("SomeNewTool", &json!({})), RiskLevel::Medium);
    }
}
