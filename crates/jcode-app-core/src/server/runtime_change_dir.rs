//! `/cd` 런타임 명령: 붙어 있는 세션의 작업 디렉터리를 바꾼다.
//! 경로는 데몬(에이전트가 도는 쪽) 파일시스템 기준으로 해석한다.
use crate::agent::Agent;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn handles(command: &str) -> bool {
    command == "cd"
}

/// 인자가 없으면 현재 디렉터리를 보여주고, 있으면 해석해서 세션에 적용한다.
pub(super) fn execute(agent: &mut Agent, args: &[String]) -> Result<(Value, String)> {
    let current = agent.working_dir().map(str::to_string);
    let input = args.join(" ");
    if input.trim().is_empty() {
        let message = match current.as_deref() {
            Some(dir) => format!("Working directory: {dir}"),
            None => "Working directory is not set.".to_string(),
        };
        return Ok((json!({"working_dir": current, "changed": false}), message));
    }

    let home = dirs::home_dir();
    let resolved = crate::change_dir::resolve_change_dir(
        &input,
        current.as_deref().map(Path::new),
        home.as_deref(),
    )
    .map_err(|message| anyhow!(message))?;
    let dir = resolved.display().to_string();
    agent.change_working_dir(&dir)?;
    let message = format!("Working directory changed to {dir}");
    Ok((json!({"working_dir": dir, "changed": true}), message))
}
