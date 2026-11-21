use std::{fs, path::Path};

use cargo_metadata::{DependencyKind, MetadataCommand};
use syn::{
    File, ItemUse, Path as RustPath, UseTree,
    visit::{self, Visit},
};

use crate::TaskResult;

fn layer(name: &str) -> Option<u8> {
    match name.strip_prefix("autopilot-")? {
        "core" => Some(0),
        "store" | "profile" | "process" => Some(1),
        "git" | "tracker" | "policy" | "accounts" => Some(2),
        "agent" | "checks" => Some(3),
        "agent-claude" | "agent-codex" | "agent-fixture" | "pipeline" => Some(4),
        "engine" => Some(5),
        "cli" | "mcp" | "ui" | "eval" => Some(6),
        _ => None,
    }
}

pub(crate) fn check() -> TaskResult {
    let metadata = MetadataCommand::new().no_deps().exec()?;
    let mut errors = Vec::new();
    for package in metadata.workspace_packages() {
        let name = package.name.as_str();
        if matches!(name, "xtask" | "autopilot-fake-agent" | "autopilot-testkit") {
            continue;
        }
        let Some(current) = layer(name) else {
            errors.push(format!("{name} has no production layer"));
            continue;
        };
        for dependency in &package.dependencies {
            if dependency.kind == DependencyKind::Development {
                continue;
            }
            let used = dependency.name.as_str();
            if layer(used).is_some_and(|next| next >= current) {
                errors.push(format!("{name} -> {used} must point to a lower layer"));
            }
            if used == "autopilot-process"
                && !matches!(
                    name,
                    "autopilot-accounts"
                        | "autopilot-git"
                        | "autopilot-tracker"
                        | "autopilot-checks"
                        | "autopilot-agent-claude"
                        | "autopilot-agent-codex"
                        | "autopilot-agent-fixture"
                )
            {
                errors.push(format!("{name} cannot use {used} directly"));
            }
            if name != "autopilot-ui" && (used == "eframe" || used.starts_with("egui")) {
                errors.push(format!("{name} cannot use {used} outside the UI"));
            }
        }
        if name != "autopilot-process" {
            let root = package
                .manifest_path
                .parent()
                .ok_or("package has no directory")?;
            let source = root.join("src");
            if source.is_dir() {
                check_sources(source.as_std_path(), &mut errors)?;
            }
            for target in &package.targets {
                if !(target.src_path.starts_with(&source)
                    || target.is_test()
                    || target.is_bench()
                    || target.is_example())
                {
                    check_source(target.src_path.as_std_path(), &mut errors)?;
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}

fn check_sources(directory: &Path, errors: &mut Vec<String>) -> TaskResult {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            check_sources(&path, errors)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            check_source(&path, errors)?;
        }
    }
    Ok(())
}

fn check_source(path: &Path, errors: &mut Vec<String>) -> TaskResult {
    let code = fs::read_to_string(path)?;
    let file = syn::parse_file(&code).map_err(|error| format!("{}: {error}", path.display()))?;
    if uses_subprocess(&file) {
        errors.push(format!(
            "{}: direct subprocess access belongs in autopilot-process",
            path.display()
        ));
    }
    Ok(())
}

fn uses_subprocess(file: &File) -> bool {
    let mut visitor = SubprocessVisitor::default();
    visitor.visit_file(file);
    visitor.found
}

#[derive(Default)]
struct SubprocessVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for SubprocessVisitor {
    fn visit_path(&mut self, path: &'ast RustPath) {
        let names: Vec<_> = path
            .segments
            .iter()
            .map(|part| part.ident.to_string())
            .collect();
        self.found |= is_subprocess(&names);
        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        self.found |= use_subprocess(&item.tree, &mut Vec::new());
    }
}

fn use_subprocess(tree: &UseTree, prefix: &mut Vec<String>) -> bool {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            let found = use_subprocess(&path.tree, prefix);
            prefix.pop();
            found
        }
        UseTree::Group(group) => group.items.iter().any(|item| use_subprocess(item, prefix)),
        UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            let found = is_subprocess(prefix);
            prefix.pop();
            found
        }
        UseTree::Rename(alias) => {
            prefix.push(alias.ident.to_string());
            let found = is_subprocess(prefix);
            prefix.pop();
            found
        }
        UseTree::Glob(_) => is_subprocess(prefix),
    }
}

fn is_subprocess(path: &[String]) -> bool {
    let [runtime, module, rest @ ..] = path else {
        return false;
    };
    if module != "process" {
        return false;
    }
    match runtime.as_str() {
        "tokio" => true,
        "std" => rest.first().is_none_or(|item| {
            matches!(
                item.as_str(),
                "self"
                    | "Command"
                    | "Child"
                    | "ChildStdin"
                    | "ChildStdout"
                    | "ChildStderr"
                    | "Stdio"
                    | "Output"
            )
        }),
        _ => false,
    }
}
