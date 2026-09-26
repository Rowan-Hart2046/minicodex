mod json;

use json::Json;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
type Result<T> = std::result::Result<T, String>;

const SYSTEM_PROMPT: &str = "You are MiniCodex, a concise coding agent working in a local project. Inspect relevant files before editing. Use tools when needed. Keep changes focused and verify your work. All paths are relative to the workspace. Never claim a tool succeeded unless its output confirms it.";

struct Options {
    prompt: Vec<String>,
    workspace: PathBuf,
    model: Option<String>,
    full_auto: bool,
}
struct Agent {
    api_key: String,
    api_base: String,
    model: String,
    workspace: PathBuf,
    full_auto: bool,
    previous_response_id: Option<String>,
}

impl Agent {
    fn run_turn(&mut self, prompt: &str) -> Result<String> {
        let mut input = Json::String(prompt.into());
        for _ in 0..20 {
            let response = self.request(input)?;
            let response_id = response.get_str("id")?.to_owned();
            let mut outputs = Vec::new();
            let mut answer = String::new();
            for item in response.get("output")?.as_array()? {
                match item.get_str("type").unwrap_or("") {
                    "function_call" => {
                        let result = self
                            .execute_tool(item.get_str("name")?, item.get_str("arguments")?)
                            .unwrap_or_else(|e| format!("ERROR: {e}"));
                        outputs.push(Json::object(vec![
                            ("type", Json::String("function_call_output".into())),
                            ("call_id", Json::String(item.get_str("call_id")?.into())),
                            ("output", Json::String(result)),
                        ]));
                    }
                    "message" => {
                        for content in item.get("content")?.as_array()? {
                            if content.get_str("type").unwrap_or("") == "output_text" {
                                answer.push_str(content.get_str("text")?);
                            }
                        }
                    }
                    _ => {}
                }
            }
            self.previous_response_id = Some(response_id);
            if outputs.is_empty() {
                return Ok(answer);
            }
            input = Json::Array(outputs);
        }
        Err("tool loop exceeded 20 steps".into())
    }

    fn request(&self, input: Json) -> Result<Json> {
        let mut fields = vec![
            ("model", Json::String(self.model.clone())),
            ("instructions", Json::String(SYSTEM_PROMPT.into())),
            ("input", input),
            ("tools", tool_definitions()),
            ("parallel_tool_calls", Json::Bool(false)),
        ];
        if let Some(id) = &self.previous_response_id {
            fields.push(("previous_response_id", Json::String(id.clone())));
        }
        let payload = Json::object(fields).stringify();
        let url = format!("{}/responses", self.api_base.trim_end_matches('/'));
        let auth = format!("Authorization: Bearer {}", self.api_key);
        let mut child = Command::new("/usr/bin/curl")
            .args([
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--max-time",
                "300",
                "-X",
                "POST",
                &url,
                "-H",
                &auth,
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start curl: {e}"))?;
        child
            .stdin
            .as_mut()
            .ok_or("curl stdin unavailable")?
            .write_all(payload.as_bytes())
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "OpenAI API request failed: {}{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ));
        }
        Json::parse(&String::from_utf8_lossy(&output.stdout))
    }

    fn execute_tool(&self, name: &str, arguments: &str) -> Result<String> {
        let args = Json::parse(arguments)?;
        eprintln!("\x1b[36;1mtool\x1b[0m {name}");
        match name {
            "list_files" => {
                let dir = self.safe_existing_path(args.get_str("path").unwrap_or("."))?;
                let mut entries = fs::read_dir(dir)
                    .map_err(|e| e.to_string())?
                    .filter_map(|e| e.ok())
                    .map(|e| {
                        format!(
                            "{}{}",
                            e.file_name().to_string_lossy(),
                            if e.path().is_dir() { "/" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>();
                entries.sort();
                Ok(entries.join("\n"))
            }
            "read_file" => {
                let path = self.safe_existing_path(args.get_str("path")?)?;
                let mut data = String::new();
                fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(200_001)
                    .read_to_string(&mut data)
                    .map_err(|e| e.to_string())?;
                if data.len() > 200_000 {
                    Err("file is larger than the 200 KB limit".into())
                } else {
                    Ok(data)
                }
            }
            "write_file" => {
                let path = self.safe_write_path(args.get_str("path")?)?;
                let content = args.get_str("content")?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(&path, content).map_err(|e| e.to_string())?;
                Ok(format!(
                    "wrote {} bytes to {}",
                    content.len(),
                    path.display()
                ))
            }
            "run_command" => {
                let command = args.get_str("command")?;
                if !self.full_auto && !confirm(command)? {
                    return Ok("command declined by user".into());
                }
                let output = Command::new("/bin/zsh")
                    .args(["-lc", command])
                    .current_dir(&self.workspace)
                    .output()
                    .map_err(|e| e.to_string())?;
                let mut result = format!(
                    "exit code: {}\n{}{}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                result.truncate(result.len().min(100_000));
                Ok(result)
            }
            _ => Err(format!("unknown tool: {name}")),
        }
    }

    fn safe_existing_path(&self, path: &str) -> Result<PathBuf> {
        let resolved = self
            .workspace
            .join(path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        ensure_inside(&self.workspace, &resolved)?;
        Ok(resolved)
    }
    fn safe_write_path(&self, path: &str) -> Result<PathBuf> {
        let candidate = self.workspace.join(path);
        let mut existing = candidate.parent().ok_or("path has no parent")?;
        while !existing.exists() {
            existing = existing.parent().ok_or("invalid parent path")?;
        }
        ensure_inside(
            &self.workspace,
            &existing.canonicalize().map_err(|e| e.to_string())?,
        )?;
        if candidate.exists() {
            ensure_inside(
                &self.workspace,
                &candidate.canonicalize().map_err(|e| e.to_string())?,
            )?;
        }
        Ok(candidate)
    }
}

fn tool_definitions() -> Json {
    Json::parse(r#"[
      {"type":"function","name":"list_files","description":"List one directory in the workspace.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}},
      {"type":"function","name":"read_file","description":"Read a UTF-8 text file in the workspace.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}},
      {"type":"function","name":"write_file","description":"Create or replace a UTF-8 text file in the workspace.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}},
      {"type":"function","name":"run_command","description":"Run a zsh command in the workspace. Use for builds, tests, git, and searches.","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"],"additionalProperties":false}}
    ]"#).expect("valid tool JSON")
}

fn ensure_inside(workspace: &Path, path: &Path) -> Result<()> {
    if path.starts_with(workspace) {
        Ok(())
    } else {
        Err("path is outside the workspace".into())
    }
}
fn confirm(command: &str) -> Result<bool> {
    eprintln!("\n\x1b[33;1mRun?\x1b[0m {command}");
    eprint!("Approve [y/N]: ");
    io::stderr().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn parse_options() -> Result<Options> {
    let mut args = env::args().skip(1);
    let mut o = Options {
        prompt: vec![],
        workspace: ".".into(),
        model: None,
        full_auto: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("minicodex {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-w" | "--workspace" => {
                o.workspace = args.next().ok_or("--workspace needs a path")?.into()
            }
            "-m" | "--model" => o.model = Some(args.next().ok_or("--model needs a value")?),
            "--full-auto" => o.full_auto = true,
            v if v.starts_with('-') => return Err(format!("unknown option: {v}")),
            v => o.prompt.push(v.into()),
        }
    }
    Ok(o)
}
fn print_help() {
    println!(
        "MiniCodex {}\n\nUSAGE:\n    minicodex [OPTIONS] [PROMPT...]\n\nOPTIONS:\n    -w, --workspace PATH   Project directory [default: .]\n    -m, --model MODEL      OpenAI model [default: gpt-6-sol]\n        --full-auto        Run shell commands without confirmation\n    -h, --help             Show this help\n    -V, --version          Show version",
        env!("CARGO_PKG_VERSION")
    );
}

fn run() -> Result<()> {
    let o = parse_options()?;
    let api_key = env::var("OPENAI_API_KEY").map_err(|_| {
        "OPENAI_API_KEY is not set. Run: export OPENAI_API_KEY='your-key'".to_string()
    })?;
    let workspace = o
        .workspace
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let model = o
        .model
        .or_else(|| env::var("MINICODEX_MODEL").ok())
        .unwrap_or_else(|| "gpt-6-sol".into());
    let mut agent = Agent {
        api_key,
        api_base: env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
        model: model.clone(),
        workspace,
        full_auto: o.full_auto,
        previous_response_id: None,
    };
    if !o.prompt.is_empty() {
        println!("{}", agent.run_turn(&o.prompt.join(" "))?);
        return Ok(());
    }
    println!(
        "\x1b[32;1mMiniCodex\x1b[0m {}  model: {model}",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type /help for commands. Ctrl-D or /exit to quit.\n");
    loop {
        print!("\x1b[32;1m>\x1b[0m ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        match line.trim() {
            "" => continue,
            "/exit" | "/quit" => break,
            "/help" => {
                println!("/help  show help\n/new   start a fresh conversation\n/exit  quit");
                continue;
            }
            "/new" => {
                agent.previous_response_id = None;
                println!("Started a new conversation.");
                continue;
            }
            prompt => match agent.run_turn(prompt) {
                Ok(a) => println!("\n{a}\n"),
                Err(e) => eprintln!("\x1b[31;1merror:\x1b[0m {e}\n"),
            },
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_outside_workspace() {
        let root = Path::new("/tmp/project");
        assert!(ensure_inside(root, Path::new("/tmp/project/src/main.rs")).is_ok());
        assert!(ensure_inside(root, Path::new("/tmp/secret")).is_err());
    }
    #[test]
    fn has_four_tools() {
        assert_eq!(tool_definitions().as_array().unwrap().len(), 4);
    }
}
