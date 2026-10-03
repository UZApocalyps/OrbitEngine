//! Build / export as one call with plain arguments: what the MCP tools
//! `build_project` and `export_game` run (live in the editor, or headless).

use std::path::{Component, Path, PathBuf};

use crate::{
    Event, ExportError, ExportOptions, ExportProject, ExportTarget, LogLevel, build, export_many,
};

/// What went wrong, in terms an AI client can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailure {
    /// The request itself is wrong (unknown target or profile, bad folder).
    BadRequest(String),
    /// The export ran and failed (invalid story, missing template...).
    Failed(String),
}

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolFailure::BadRequest(m) | ToolFailure::Failed(m) => f.write_str(m),
        }
    }
}

/// A folder inside the project: relative, without `..`.
fn inside_project(root: &Path, relative: &str) -> Result<PathBuf, ToolFailure> {
    let path = Path::new(relative);
    let escapes = path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        });
    if relative.trim().is_empty() || escapes {
        return Err(ToolFailure::BadRequest(format!(
            "out_dir `{relative}` must be a folder inside the project (relative, without `..`)"
        )));
    }
    Ok(root.join(path))
}

fn mib(bytes: u64) -> String {
    format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// Builds the pak (`target` `None`) or exports for `target`
/// (`windows`, `linux` or `web`) the project at `project_path` (a project
/// folder or a story file), and describes the result in a few lines.
///
/// `out_dir` is relative to the project folder (default `build` for a
/// build, `export` for an export). The project is read from disk: the caller
/// saves its unsaved edits first.
pub fn run_request(
    project_path: &Path,
    build_only: bool,
    target: Option<&str>,
    profile: Option<&str>,
    out_dir: Option<&str>,
    options: &ExportOptions,
) -> Result<String, ToolFailure> {
    let project =
        ExportProject::open(project_path).map_err(|e| ToolFailure::Failed(e.to_string()))?;
    let profile = project
        .profile(profile)
        .map_err(|e| ToolFailure::BadRequest(e.to_string()))?;
    let default_dir = if build_only { "build" } else { "export" };
    let out = inside_project(&project.root, out_dir.unwrap_or(default_dir))?;
    let mut journal: Vec<String> = Vec::new();
    let mut progress = |event: &Event| {
        if let Event::Log {
            level: LogLevel::Warning,
            text,
        } = event
        {
            journal.push(format!("warning: {text}"));
        }
    };
    let failed = |e: ExportError| ToolFailure::Failed(e.to_string());

    if build_only {
        let report = build(&project, &profile, &out, options, &mut progress).map_err(failed)?;
        let mut text = format!(
            "Built {} ({} file(s), {}; {} before compression).",
            report.pak_path.display(),
            report.pak.entries,
            mib(report.pak.pak_bytes),
            mib(report.pak.raw_bytes)
        );
        append_warnings(&mut text, &journal);
        return Ok(text);
    }

    let target_name = target.ok_or_else(|| {
        ToolFailure::BadRequest("a target is needed (windows, linux, web or android)".to_owned())
    })?;
    let target = ExportTarget::parse(target_name).ok_or_else(|| {
        ToolFailure::BadRequest(format!(
            "unknown target `{target_name}` (windows, linux, web or android)"
        ))
    })?;
    let reports =
        export_many(&project, &profile, &[target], &out, options, &mut progress).map_err(failed)?;
    let report = &reports[0];
    let mut text = format!(
        "Exported {} to {} ({}, profile `{}`).",
        target.id(),
        report.output.display(),
        mib(report.bytes),
        report.profile
    );
    if target == ExportTarget::Web {
        text.push_str(" Serve the folder over http (for instance `orbit export web --serve`) to play it in a browser.");
    }
    append_warnings(&mut text, &journal);
    Ok(text)
}

fn append_warnings(text: &mut String, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    text.push_str(&format!("\n{} warning(s):", warnings.len()));
    for warning in warnings {
        text.push_str(&format!("\n- {warning}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Templates;
    use crate::testing::{TempDir, demo_project};

    fn options_with_linux_template(dir: &Path) -> ExportOptions {
        std::fs::create_dir_all(dir.join("linux")).unwrap();
        std::fs::write(dir.join("linux/orbit-player"), b"fake").unwrap();
        ExportOptions {
            templates: Templates::new(vec![dir.to_owned()]),
            cancel: None,
            android: Default::default(),
        }
    }

    #[test]
    fn builds_and_exports_inside_the_project() {
        let project = TempDir::new("tool_project");
        let templates = TempDir::new("tool_templates");
        demo_project(project.path());
        let options = options_with_linux_template(templates.path());

        let built = run_request(project.path(), true, None, None, None, &options).unwrap();
        assert!(built.starts_with("Built "), "{built}");
        assert!(project.path().join("build/demo.pak").is_file());
        assert!(built.contains("warning(s)"), "{built}");

        let exported = run_request(
            project.path(),
            false,
            Some("linux"),
            None,
            Some("dist"),
            &options,
        )
        .unwrap();
        assert!(exported.contains("Exported linux"), "{exported}");
        assert!(
            project
                .path()
                .join("dist/demo-1.0.0-linux.tar.gz")
                .is_file()
        );
    }

    #[test]
    fn bad_requests_are_told_apart_from_failures() {
        let project = TempDir::new("tool_bad");
        let templates = TempDir::new("tool_bad_templates");
        demo_project(project.path());
        let options = options_with_linux_template(templates.path());
        let bad = |target: Option<&str>, profile: Option<&str>, out: Option<&str>| {
            run_request(project.path(), false, target, profile, out, &options).unwrap_err()
        };
        assert!(matches!(
            bad(Some("mac"), None, None),
            ToolFailure::BadRequest(_)
        ));
        assert!(matches!(bad(None, None, None), ToolFailure::BadRequest(_)));
        assert!(matches!(
            bad(Some("linux"), Some("nope"), None),
            ToolFailure::BadRequest(_)
        ));
        for escape in ["../out", "/tmp/out", "a/../../b", ""] {
            assert!(
                matches!(
                    bad(Some("linux"), None, Some(escape)),
                    ToolFailure::BadRequest(_)
                ),
                "{escape}"
            );
        }
        // A missing template is a failure of the export, said clearly.
        let failure = bad(Some("windows"), None, None);
        assert!(
            matches!(&failure, ToolFailure::Failed(m) if m.contains("template")),
            "{failure}"
        );
    }
}
