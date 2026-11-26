//! What each AI the app runs can do, in one table, so the rest of the app (and the page, through `/api/status`)
//! asks the table instead of naming AIs. Each entry is as measured or read for that AI's integration; DEVELOPING.md
//! has the evidence. Tests pin the table, so a change to it is a decision, not a side effect.

use serde_json::{Value, json};

use crate::paths;

/// The run modes an AI keeps to by its own rules (its permission settings and flags), without the app watching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    /// Ask me: only the user's allowed programs run, file tools write only in the chat's folders.
    pub ask: bool,
    /// Full auto (and Overnight): nothing asks, the app's own files are still refused.
    pub full: bool,
    /// Studio: Full auto for a Lead and its experts.
    pub studio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub id: &'static str,
    /// Can run an expert or helper (a Studio team, a Lead's job).
    pub experts: bool,
    /// Offered in the page's AI menus (Codex is kept for chats saved on it, and is not offered for new ones).
    pub pickable: bool,
    /// Picks up a saved conversation (Claude `--resume`, Antigravity `--conversation`, Codex `exec resume`, Cursor
    /// `--resume`, OpenCode's session).
    pub resume: bool,
    /// Reports tokens or cost per turn (the usage log).
    pub usage: bool,
    /// Reports its plan's usage limits (`limits`: Claude Code's probe and `rate_limit_event`, Codex's app-server).
    pub limits: bool,
    /// Returns pictures from tools, which the chat shows.
    pub images: bool,
    /// Has a sandbox of its own the app uses (Codex `workspace-write` in Ask me, Cursor's in Full auto).
    pub sandbox: bool,
    /// Asks the app live for permission and waits for the answer (OpenCode's `permission.asked`); the others refuse
    /// by their rules and report it afterwards.
    pub live_permissions: bool,
    /// Takes the app's effort setting (Cursor's and OpenCode's model ids carry their own).
    pub effort: bool,
    /// Its model menu is grouped by provider ("OpenCode Go · glm-5.3").
    pub grouped_models: bool,
    /// Keeps one process open per chat and takes messages and Stop over its own protocol (Claude Code's stream-json
    /// input, `claude_live`; Gemini and Cursor over ACP, `agy acp` and `cursor-agent acp`, `acp`), with a process
    /// per turn as the fallback; the others run a process per turn, or on the app's own server (OpenCode). Whether
    /// an AI that can be kept open is, by default, is `agents::KEPT_ON_REQUEST` (Gemini and Cursor are not yet: see
    /// DEVELOPING.md), and Cursor's is for Ask me only (`agents::CursorLaunch`). `usage` is what a process per
    /// message reports: a Cursor turn on its kept process reports none, and is logged as not reported.
    pub live: bool,
    /// The longest a call to a tool server may run on this AI before the AI cuts it off, in seconds, on a process per
    /// message; None where no such limit was measured under the 15 minutes the app asks for (`MCP_TOOL_TIMEOUT`).
    /// Measured: Cursor 60 s. The app's tool server is told a budget under it (`call_budget`) and answers in time
    /// (`rbxkit_mcp.py`, "long calls").
    pub tool_call_limit: Option<u32>,
    /// The same on the AI's kept process. Measured: Cursor 60 s, Gemini (`agy acp`) three minutes.
    pub tool_call_limit_kept: Option<u32>,
    pub modes: Modes,
}

const ALL_MODES: Modes = Modes { ask: true, full: true, studio: true };

/// The table. Every AI keeps to every mode today: by its own rules (Claude Code's permission modes and deny rules,
/// Antigravity's settings and `--dangerously-skip-permissions` on a process per message, Codex's sandbox and bypass,
/// Cursor's allow list and sandbox, OpenCode's permission rules), or, for Gemini on its kept process, by the app
/// answering each of its permission requests (`acp::decide`); a mode one cannot keep to is set false here and is
/// then not offered.
pub const TABLE: [Caps; 5] = [
    Caps { id: "claude", experts: true, pickable: true, resume: true, usage: true, limits: true, images: true, sandbox: false, live_permissions: false, effort: true, grouped_models: false, live: true, tool_call_limit: None, tool_call_limit_kept: None, modes: ALL_MODES },
    Caps { id: "gemini", experts: true, pickable: true, resume: true, usage: true, limits: false, images: true, sandbox: false, live_permissions: false, effort: true, grouped_models: false, live: true, tool_call_limit: None, tool_call_limit_kept: Some(180), modes: ALL_MODES },
    Caps { id: "chatgpt", experts: true, pickable: false, resume: true, usage: true, limits: true, images: true, sandbox: true, live_permissions: false, effort: true, grouped_models: false, live: false, tool_call_limit: None, tool_call_limit_kept: None, modes: ALL_MODES },
    Caps { id: "cursor", experts: true, pickable: true, resume: true, usage: true, limits: false, images: true, sandbox: true, live_permissions: false, effort: false, grouped_models: false, live: true, tool_call_limit: Some(60), tool_call_limit_kept: Some(60), modes: ALL_MODES },
    Caps { id: "opencode", experts: true, pickable: true, resume: true, usage: true, limits: false, images: true, sandbox: false, live_permissions: true, effort: false, grouped_models: true, live: false, tool_call_limit: None, tool_call_limit_kept: None, modes: ALL_MODES },
];

/// An AI's entry (Claude's for an unknown id: chats saved before there was a choice were all Claude's).
pub fn of(ai: &str) -> Caps {
    TABLE.iter().copied().find(|c| c.id == ai).unwrap_or(TABLE[0])
}

/// What the app's tool server is told a call may take on this AI and path (`RBXKIT_CALL_BUDGET`, seconds): the AI's
/// limit with a safety margin taken off (a sixth, at least 15 s: Cursor 60 → 45, Gemini kept 180 → 150), so the
/// answer is with the AI well before it gives up. None: no limit, and the tool server behaves as ever.
pub fn call_budget(ai: &str, kept: bool) -> Option<u32> {
    let c = of(ai);
    budget_under(if kept { c.tool_call_limit_kept } else { c.tool_call_limit })
}

fn budget_under(limit: Option<u32>) -> Option<u32> {
    limit.map(|l| l.saturating_sub((l / 6).max(15)).max(1))
}

/// Whether this computer offers the AI (Cursor and OpenCode are not offered on Windows yet).
pub fn available(ai: &str) -> bool {
    paths::AIS.contains(&ai)
}

/// The AIs an expert can be put on in Setup > Experts: offered here, pickable, and able to run experts.
pub fn expert_ais() -> Vec<&'static str> {
    paths::AIS.iter().copied().filter(|a| of(a).experts && of(a).pickable).collect()
}

/// The table for the page (`/api/status` → `ai.caps`), with whether this computer offers each.
pub fn table() -> Value {
    let mut out = serde_json::Map::new();
    for c in TABLE {
        out.insert(
            c.id.into(),
            json!({"experts": c.experts, "pickable": c.pickable, "resume": c.resume, "usage": c.usage, "limits": c.limits, "images": c.images,
                   "sandbox": c.sandbox, "live_permissions": c.live_permissions, "effort": c.effort, "grouped_models": c.grouped_models, "live": c.live,
                   "tool_call_limit": c.tool_call_limit, "tool_call_limit_kept": c.tool_call_limit_kept,
                   "modes": {"ask": c.modes.ask, "full": c.modes.full, "studio": c.modes.studio}, "available": available(c.id)}),
        );
    }
    Value::Object(out)
}

/// The mode a message runs in on this AI: the one asked for when the AI keeps to it, else Ask me (and a note for the
/// chat). `full` and `studio` as the message asked; returns (full, studio, note).
pub fn mode_for(ai: &str, full: bool, studio: bool) -> (bool, bool, Option<&'static str>) {
    fall_back(of(ai).modes, full, studio)
}

fn fall_back(m: Modes, full: bool, studio: bool) -> (bool, bool, Option<&'static str>) {
    if studio && !m.studio {
        return (false, false, Some("This AI cannot keep to Studio's rules by itself, so this message went as Ask me."));
    }
    if full && !m.full {
        return (false, false, Some("This AI cannot keep to Full auto's rules by itself, so this message went as Ask me."));
    }
    (full, studio, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_pinned() {
        let row = |id: &str| {
            let c = of(id);
            [c.experts, c.pickable, c.resume, c.usage, c.limits, c.images, c.sandbox, c.live_permissions, c.effort, c.grouped_models, c.live, c.modes.ask, c.modes.full, c.modes.studio]
        };
        //                       experts pick  resume usage limits images sandbox asks   effort group  live   ask   full  studio
        assert_eq!(row("claude"), [true, true, true, true, true, true, false, false, true, false, true, true, true, true]);
        assert_eq!(row("gemini"), [true, true, true, true, false, true, false, false, true, false, true, true, true, true]);
        assert_eq!(row("chatgpt"), [true, false, true, true, true, true, true, false, true, false, false, true, true, true]);
        assert_eq!(row("cursor"), [true, true, true, true, false, true, true, false, false, false, true, true, true, true]);
        assert_eq!(row("opencode"), [true, true, true, true, false, true, false, true, false, true, false, true, true, true]);
        assert_eq!(TABLE.len(), 5);
        assert_eq!(of("unknown").id, "claude");
        // the longest a tool call may run, per message and kept, as measured: only Cursor (both) and Gemini's kept process
        let limits = |id: &str| (of(id).tool_call_limit, of(id).tool_call_limit_kept);
        assert_eq!(limits("claude"), (None, None));
        assert_eq!(limits("gemini"), (None, Some(180)));
        assert_eq!(limits("chatgpt"), (None, None));
        assert_eq!(limits("cursor"), (Some(60), Some(60)));
        assert_eq!(limits("opencode"), (None, None));
    }

    #[test]
    fn the_tool_servers_budget_is_the_limit_with_a_margin_off() {
        assert_eq!((call_budget("cursor", false), call_budget("cursor", true)), (Some(45), Some(45)));
        assert_eq!((call_budget("gemini", false), call_budget("gemini", true)), (None, Some(150)));
        for ai in ["claude", "opencode", "chatgpt"] {
            assert_eq!((call_budget(ai, false), call_budget(ai, true)), (None, None), "{ai}");
        }
        assert_eq!(budget_under(Some(10)), Some(1), "never zero: zero would mean no limit");
        assert_eq!(table()["cursor"]["tool_call_limit"], 60);
        assert_eq!(table()["gemini"]["tool_call_limit_kept"], 180);
        assert!(table()["claude"]["tool_call_limit"].is_null());
    }

    #[test]
    fn every_offered_ai_has_an_entry_and_the_page_gets_them_all() {
        for ai in paths::AIS {
            assert!(TABLE.iter().any(|c| c.id == *ai), "{ai}");
        }
        let t = table();
        assert_eq!(t["opencode"]["modes"]["full"], true);
        assert_eq!(t["claude"]["limits"], true);
        assert_eq!(t["cursor"]["available"], json!(!cfg!(windows)));
        assert!(!expert_ais().contains(&"chatgpt"));
    }

    #[test]
    fn a_mode_an_ai_cannot_keep_to_falls_back_to_ask_me() {
        assert_eq!(mode_for("claude", true, true), (true, true, None));
        assert_eq!(mode_for("opencode", true, false), (true, false, None));
        // the rule itself, on made-up entries (none of today's AIs lacks a mode)
        let no_full = Modes { ask: true, full: false, studio: false };
        assert!(matches!(fall_back(no_full, true, false), (false, false, Some(n)) if n.contains("Full auto")));
        assert!(matches!(fall_back(no_full, true, true), (false, false, Some(n)) if n.contains("Studio")));
        assert_eq!(fall_back(no_full, false, false), (false, false, None));
        let no_studio = Modes { ask: true, full: true, studio: false };
        assert!(matches!(fall_back(no_studio, true, true), (false, false, Some(_))));
        assert_eq!(fall_back(no_studio, true, false), (true, false, None));
    }
}
