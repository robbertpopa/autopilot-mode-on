//! Tools worth knowing about for a project, chosen from what the scan found.
//! This is advice: nothing here installs, enables or runs anything. Agent
//! steps are launched with `--strict-mcp-config` and a restricted tool list
//! because ticket text is untrusted, and a suggestion does not change that.
//!
//! The list is data kept in this file and changed by a pull request. Each
//! entry names its source, maintainer, licence and the release it was looked
//! at, and says plainly that it has not been reviewed by us: the owner reads
//! the source and the permissions before using any of it.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{Error, ProjectCard, rules::project_key};

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

const ALL: [ToolSuggestion; 4] = [CONTEXT7, PLAYWRIGHT, GITHUB, AZURE];
const ALLOWED_FILE: &str = "allowed-tools.txt";

fn allowed_path(directory: &Path, workspace: &str) -> PathBuf {
    directory
        .join("projects")
        .join(project_key(workspace))
        .join(ALLOWED_FILE)
}

/// The tools the owner marked as allowed for a project. This is the owner's
/// note only: steps still run without MCP servers, and nothing reads this list
/// until an MCP launch path exists. Names that are no longer in the catalog
/// are dropped.
///
/// # Errors
/// Returns an error only when the file exists but cannot be read.
pub fn allowed_tools(directory: &Path, workspace: &str) -> Result<Vec<String>, Error> {
    let text = match fs::read_to_string(allowed_path(directory, workspace)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    Ok(text
        .lines()
        .filter(|line| ALL.iter().any(|tool| tool.name == *line))
        .map(str::to_owned)
        .collect())
}

/// Mark a catalog tool as allowed, or not, for a project.
///
/// # Errors
/// Returns an error for a name that is not in the catalog, or when the file
/// cannot be written.
pub fn set_tool_allowed(
    directory: &Path,
    workspace: &str,
    name: &str,
    allowed: bool,
) -> Result<(), Error> {
    if !ALL.iter().any(|tool| tool.name == name) {
        return Err(Error::Card(format!(
            "{name:?} is not a tool in the catalog"
        )));
    }
    let mut names = allowed_tools(directory, workspace)?;
    names.retain(|kept| kept != name);
    if allowed {
        names.push(name.to_owned());
    }
    names.sort();
    let path = allowed_path(directory, workspace);
    let folder = path
        .parent()
        .ok_or_else(|| Error::Card("no folder".into()))?;
    fs::create_dir_all(folder)?;
    let temporary = folder.join(format!("{ALLOWED_FILE}.tmp"));
    let mut text = names.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    fs::write(&temporary, text)?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

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

    // Invariant: only catalog names are kept, one project's choice does not
    // leak into another, and unallowing removes the name. A change that kept
    // free text would let a hand-edited file put anything in the list.
    #[test]
    fn the_allow_list_keeps_catalog_names_per_project() {
        let data = tempfile::tempdir().unwrap();
        assert!(set_tool_allowed(data.path(), "D:/w/a", "Rm Everything", true).is_err());
        set_tool_allowed(data.path(), "D:/w/a", "Context7", true).unwrap();
        set_tool_allowed(data.path(), "D:/w/a", "Playwright MCP", true).unwrap();
        assert_eq!(
            allowed_tools(data.path(), "D:/w/a").unwrap(),
            vec!["Context7", "Playwright MCP"]
        );
        assert_eq!(
            allowed_tools(data.path(), "D:/w/b").unwrap(),
            Vec::<String>::new()
        );
        set_tool_allowed(data.path(), "D:/w/a", "Context7", false).unwrap();
        assert_eq!(
            allowed_tools(data.path(), "D:/w/a").unwrap(),
            vec!["Playwright MCP"]
        );
        fs::write(
            allowed_path(data.path(), "D:/w/a"),
            "Playwright MCP\nevil\n",
        )
        .unwrap();
        assert_eq!(
            allowed_tools(data.path(), "D:/w/a").unwrap(),
            vec!["Playwright MCP"]
        );
    }
}
