//! Which tools a client is shown (port of T3 Code's McpToolExposure.ts). Stub: Task 3 fills tool_search/tool_call.

use super::ToolDef;

/// How a client takes the tools: all of them, or (Antigravity) only a few plus tool_search/tool_call.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Full,
    DeferredProxy,
}

/// The mode a `fridae-tool-exposure` header asks for.
pub fn mode_of(header: &str) -> Mode {
    if header.trim().eq_ignore_ascii_case("deferred-proxy") { Mode::DeferredProxy } else { Mode::Full }
}

/// Whether tools/list shows the tool in this mode.
pub fn listed(_mode: Mode, name: &str) -> bool {
    name != "tool_search" && name != "tool_call"
}

/// The deferred tools (tool_search, tool_call).
pub fn defs() -> Vec<ToolDef> {
    vec![]
}
