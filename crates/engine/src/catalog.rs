//! Tools worth knowing about for a project, chosen from what the scan found.
//! This is advice: nothing here installs, enables or runs anything. Agent
//! steps are launched with `--strict-mcp-config` and a restricted tool list
//! because ticket text is untrusted, and a suggestion does not change that.
//!
//! The list is data kept in this file and changed by a pull request. Each
//! entry names its source, maintainer, licence and the release it was looked
//! at, and says plainly that it has not been reviewed by us: the owner reads
//! the source and the permissions before using any of it.

use crate::ProjectCard;

/// One suggested skill or MCP server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolSuggestion {
    pub name: &'static str,
    /// `mcp` or `skill`.
    pub kind: &'static str,
    pub summary: &'static str,
    pub source: &'static str,
    pub maintainer: &'static str,
    pub licence: &'static str,
    /// The release looked at on `checked_on`, not a promise it is current.
    pub pinned: &'static str,
    pub checked_on: &'static str,
    /// What it can do once the owner enables it.
    pub permissions: &'static str,
    /// Always false for now: no entry has had a code review by this project.
    pub reviewed: bool,
}

const CONTEXT7: ToolSuggestion = ToolSuggestion {
    name: "Context7",
    kind: "mcp",
    summary: "Looks up current library documentation instead of relying on memory.",
    source: "https://github.com/upstash/context7",
    maintainer: "Upstash",
    licence: "MIT",
    pinned: "@upstash/context7-mcp 4.7.3",
    checked_on: "2026-11-23",
    permissions: "Sends library names and questions to an online service; reads nothing in the project.",
    reviewed: false,
};

const PLAYWRIGHT: ToolSuggestion = ToolSuggestion {
    name: "Playwright MCP",
    kind: "mcp",
    summary: "Drives a real browser, to check a web page or a UI change.",
    source: "https://github.com/microsoft/playwright-mcp",
    maintainer: "Microsoft",
    licence: "Apache-2.0",
    pinned: "@playwright/mcp 0.0.86",
    checked_on: "2026-11-23",
    permissions: "Opens and controls a browser, reaches any site it is pointed at and can read pages it loads.",
    reviewed: false,
};

const GITHUB: ToolSuggestion = ToolSuggestion {
    name: "GitHub MCP server",
    kind: "mcp",
    summary: "Reads and changes issues, pull requests and repository files on GitHub.",
    source: "https://github.com/github/github-mcp-server",
    maintainer: "GitHub",
    licence: "MIT",
    pinned: "v2.2.0",
    checked_on: "2026-11-23",
    permissions: "Needs a GitHub token. Autopilot never gives an agent step that token, so use it only in your own Claude sessions.",
    reviewed: false,
};

const AZURE: ToolSuggestion = ToolSuggestion {
    name: "Azure MCP Server",
    kind: "mcp",
    summary: "Works with Azure resources and Azure DevOps from an agent.",
    source: "https://github.com/microsoft/mcp",
    maintainer: "Microsoft",
    licence: "MIT",
    pinned: "Azure.Mcp.Server 4.0.0-beta.10 (a beta)",
    checked_on: "2026-11-23",
    permissions: "Acts with your Azure login: it can read and change resources you can. Use only in your own sessions.",
    reviewed: false,
};

/// The suggestions for a scanned project, in a fixed order.
#[must_use]
pub fn suggest_tools(card: &ProjectCard) -> Vec<ToolSuggestion> {
    let mut found = Vec::new();
    if !card.stack.is_empty() {
        found.push(CONTEXT7);
    }
    if card.stack.iter().any(|stack| stack.starts_with("Node")) {
        found.push(PLAYWRIGHT);
    }
    // GitHub workflow files are the only CI names that are not one of the two
    // fixed file names of the other systems.
    let github = card
        .ci
        .iter()
        .any(|name| name != "azure-pipelines.yml" && name != ".gitlab-ci.yml");
    if github {
        found.push(GITHUB);
    }
    if card.ci.iter().any(|name| name == "azure-pipelines.yml") {
        found.push(AZURE);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(stack: &[&str], ci: &[&str]) -> ProjectCard {
        ProjectCard {
            stack: stack.iter().map(|s| (*s).to_owned()).collect(),
            ci: ci.iter().map(|s| (*s).to_owned()).collect(),
            ..ProjectCard::default()
        }
    }

    fn names(card: &ProjectCard) -> Vec<&'static str> {
        suggest_tools(card).iter().map(|tool| tool.name).collect()
    }

    // Invariant: suggestions follow what the scan found and nothing else: an
    // empty project gets none, a Rust project gets only the general one, and
    // each extra needs its own sign. A rule that suggested everything would
    // teach the owner to ignore the list.
    #[test]
    fn suggestions_follow_the_scan() {
        assert_eq!(names(&ProjectCard::default()), Vec::<&str>::new());
        assert_eq!(names(&card(&["Rust"], &[])), ["Context7"]);
        assert_eq!(
            names(&card(&["Node (pnpm)"], &["build.yml"])),
            ["Context7", "Playwright MCP", "GitHub MCP server"]
        );
        assert_eq!(
            names(&card(&["Go"], &["azure-pipelines.yml"])),
            ["Context7", "Azure MCP Server"]
        );
        assert_eq!(names(&card(&["Go"], &[".gitlab-ci.yml"])), ["Context7"]);
    }

    // Invariant: every entry names where it came from, what it can do and the
    // release that was looked at, and none claims a review it did not get. An
    // entry added without these would be advice the owner cannot check.
    #[test]
    fn every_entry_can_be_checked_by_the_owner() {
        for tool in [CONTEXT7, PLAYWRIGHT, GITHUB, AZURE] {
            assert!(
                tool.source.starts_with("https://github.com/"),
                "{}",
                tool.name
            );
            assert!(!tool.pinned.is_empty() && !tool.permissions.is_empty());
            assert!(!tool.maintainer.is_empty() && !tool.licence.is_empty());
            assert_eq!(tool.checked_on.len(), 10);
            assert!(!tool.reviewed, "{} claims a review", tool.name);
        }
    }
}
