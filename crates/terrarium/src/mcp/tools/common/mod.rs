//! Tools every terrarium project gets, whatever its toolchain: build-runner
//! passthroughs, file inspection, and the sandbox-violation log.

pub(crate) mod command;
pub(crate) mod files;
pub(crate) mod runners;
pub(crate) mod spenn;
pub(crate) mod violations;

use std::path::Path;

pub(crate) use runners::DockerRunInput;

use super::inputs::NoInput;
use super::schema::schema_for;
use super::types::ToolDefinition;

/// Raises Claude Code's ~120s default MCP tool-call timeout (ms).
pub const MCP_TOOL_TIMEOUT_MS: u64 = 600_000;

pub(crate) fn common_definitions(_project_root: &Path) -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "report_violation",
            description: "Report a sandbox violation when an operation is denied by the macOS sandbox. Call this whenever you encounter a permission error so the violation is logged to violations.md",
            input_schema: schema_for::<violations::ReportViolationInput>(),
        },
        ToolDefinition {
            name: "list_files",
            description: "List files matching a glob pattern within the project. Returns relative paths, up to 500 results.",
            input_schema: schema_for::<files::ListFilesInput>(),
        },
        ToolDefinition {
            name: "directory_structure",
            description: "Show a tree-like directory structure for the project or a sub-path, up to depth 5. Excludes hidden state and build output directories.",
            input_schema: schema_for::<files::DirectoryStructureInput>(),
        },
        ToolDefinition {
            name: "search_files",
            description: "Search file contents using ripgrep (rg). Returns matches in file:line:content format, capped at 4000 chars. Requires ripgrep installed.",
            input_schema: schema_for::<files::SearchFilesInput>(),
        },
        ToolDefinition {
            name: "create_directory",
            description: "Create a directory (and any missing parents) within the project. Path must be relative to the project root.",
            input_schema: schema_for::<files::CreateDirectoryInput>(),
        },
        ToolDefinition {
            name: "make_install",
            description: "Run `make install` in the project root or a subfolder. Use `path` to target a subdirectory that contains a Makefile.",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "make_run",
            description: "Run `make` directly with structured arguments. Output is always written live to a log. Use `args` for make flags/variables, optional `target` for the target, optional `background` for long-running commands, and optional `output_regex` to return only matching output lines.",
            input_schema: schema_for::<runners::MakeRunInput>(),
        },
        ToolDefinition {
            name: "just_run",
            description: "Run `just` directly with structured arguments. Output is always written live to a log. Use `args` for just flags/variables, optional `recipe` for the recipe (with recipe arguments appended to `args`), optional `background` for long-running commands, and optional `output_regex` to return only matching output lines.",
            input_schema: schema_for::<runners::JustRunInput>(),
        },
        ToolDefinition {
            name: "docker_run",
            description: "Run `docker` directly with structured arguments. Output is always written live to a log. Use `args` for argv after docker, for example [\"compose\", \"logs\", \"service\", \"--tail\", \"1000\"]. Use optional `background` for long-running commands and optional `output_regex` instead of shell pipes or grep.",
            input_schema: schema_for::<DockerRunInput>(),
        },
        ToolDefinition {
            name: "command_log_read",
            description: "Read or tail a live Make/Just/Docker command log returned by make_install, make_run, just_run, or docker_run. Supports optional regex filtering.",
            input_schema: schema_for::<command::CommandLogReadInput>(),
        },
        ToolDefinition {
            name: "spenn_list_scenarios",
            description: "Run `spenn-api-client list-scenarios` and return the available scenarios.",
            input_schema: schema_for::<spenn::SpennListScenariosInput>(),
        },
        ToolDefinition {
            name: "spenn_run_scenario",
            description: "Run `spenn-api-client run-scenario <name>` against http://localhost:3003.",
            input_schema: schema_for::<spenn::SpennRunScenarioInput>(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_schemas_expose_path_and_project() {
        let defs = common_definitions(Path::new("."));
        let check = |name: &str| {
            let def = defs.iter().find(|d| d.name == name).expect(name);
            let props = def.input_schema.get("properties").expect("has properties");
            assert!(props.get("path").is_some(), "{name}: missing 'path'");
            assert!(props.get("project").is_some(), "{name}: missing 'project'");
        };
        check("report_violation");
        check("list_files");
        check("directory_structure");
        check("search_files");
        check("create_directory");
        check("make_install");
        check("make_run");
        check("just_run");
        check("docker_run");
    }

    #[test]
    fn spenn_schemas_expose_project_without_path() {
        let defs = common_definitions(Path::new("."));
        let check = |name: &str| {
            let def = defs.iter().find(|d| d.name == name).expect(name);
            let props = def.input_schema.get("properties").expect("has properties");
            assert!(props.get("project").is_some(), "{name}: missing 'project'");
            assert!(props.get("path").is_none(), "{name}: unexpected 'path'");
        };
        check("spenn_list_scenarios");
        check("spenn_run_scenario");
    }
}
