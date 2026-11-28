//! What each AI is told about the toolbox (port of T3 Code's orchestrationInstructions.ts, adapted to Fridae's tools:
//! thread -> chat, provider -> AI, T3 Code -> Fridae, and only the parts naming tools Fridae has).

// Codex cuts developer-instruction entries over 4,000 bytes: each section stands alone and stays under that.
const SECTION_ONE: &str = "## Fridae toolbox

Ordinary coding tasks need no Fridae tools.

- The `fridae` tools run the Fridae app itself: chats, projects, AIs and models, settings and the theme. Use them when the user asks you to do something with the app.
- Call `fridae_capabilities` once per chat, before you pick an AI or model. Again only after an error or a user-named AI change. AI or model unavailable: say so. Never switch unasked.
- As a subagent or child, do the work yourself. Start no subagents unless your task says to.";

const SECTION_TWO: &str = "- `chat_launch` makes a new top-level chat, only when the user asks for a new chat. \"Subagent\", \"delegate\", and \"parallel\" never mean one.
- After `chat_launch` or `chat_send`, answer with its `link` right away: the chat's own work is that chat's. Wait for or read a chat you launched only when the user asks what it said or your task needs its result.
- Given a chat id, `chat_read` it before you answer about it.
- In your reply, mention a chat by its `link`, verbatim, never a plain title or bare id. The same for a project.
- Restyle Fridae with `theme_set` only when the user asks.
- Some changes show the user an Allow or Don't card in this chat and wait for the answer. Declined: don't ask again in chat; say what's blocked and go on without it.
- Fridae tools may load late. Try the name once before you call it absent.";

const DEFERRED_PROXY: &str = "Deferred Fridae tools: `fridae_capabilities`, `chat_launch`, `chat_send`, `chat_list`, `project_list`, `tool_search` and `tool_call` are declared directly. Call those directly, with no search first. For any other Fridae tool (`chat_*`, `project_*`, `settings_*`, `theme_*`), call `tool_search` to get its schema and `tool_call({ name, arguments })` to run it.";

/// The instruction sections, each under 4,000 bytes; `deferred` adds how to reach the tools not listed directly.
pub fn sections(deferred: bool) -> Vec<&'static str> {
    let mut all = vec![SECTION_ONE, SECTION_TWO];
    if deferred {
        all.push(DEFERRED_PROXY);
    }
    all
}

/// The instructions for an AI that takes standing instructions of its own (Claude's `--append-system-prompt`, Codex's
/// developer instructions, OpenCode's session instructions): a blank line, the sections, a newline (T3's form).
pub fn system_text() -> String {
    format!("

{}
", sections(false).join("

"))
}

/// A prompt with the instructions in front, for AIs whose instructions ride on the prompt (ACP), with the
/// deferred-tools section when the AI takes the deferred tools (Gemini). A prompt that starts with `/` is a slash
/// command, which must stay at the start: it is left alone.
pub fn prompt_prefix_for(prompt: &str, deferred: bool) -> String {
    if prompt.trim_start().starts_with('/') {
        return prompt.to_string();
    }
    format!("<fridae_toolbox_instructions>
{}
</fridae_toolbox_instructions>

<user_request>
{prompt}
</user_request>", sections(deferred).join("

"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_is_under_4000_bytes() {
        assert!(!sections(true).is_empty());
        for deferred in [false, true] {
            for s in sections(deferred) {
                assert!(s.len() < 4000, "{} bytes", s.len());
            }
        }
    }

    #[test]
    fn section_one_opens_with_t3s_line() {
        let one = sections(false)[0];
        assert!(one.starts_with("## Fridae toolbox\n\nOrdinary coding tasks need no Fridae tools."), "{one}");
        assert!(one.contains("As a subagent or child, do the work yourself."));
        assert!(sections(false)[1].ends_with("Fridae tools may load late. Try the name once before you call it absent."));
        assert!(system_text().contains("Ordinary coding tasks need no Fridae tools."));
        assert!(!system_text().contains("T3"));
    }

    #[test]
    fn deferred_adds_the_proxy_section() {
        assert_eq!(sections(false).len(), 2);
        let d = sections(true);
        assert_eq!(d.len(), 3);
        assert!(d[2].starts_with("Deferred Fridae tools: `fridae_capabilities`, `chat_launch`, `chat_send`, `chat_list`, `project_list`, `tool_search` and `tool_call` are declared directly."), "{}", d[2]);
        assert!(d[2].contains("Call those directly"));
        assert!(!system_text().contains("Deferred Fridae tools"));
        assert!(prompt_prefix_for("hi", true).contains("Deferred Fridae tools"));
        assert!(!prompt_prefix_for("hi", false).contains("Deferred Fridae tools"));
    }

    #[test]
    fn a_launched_chat_is_not_waited_for_unasked() {
        let all = sections(true).join("
");
        assert!(!all.contains("Follow it with"), "{all}");
        assert!(all.contains("Wait for or read a chat you launched only when the user asks what it said or your task needs its result"), "{all}");
        assert!(all.contains("answer with its `link` right away"), "{all}");
    }

    #[test]
    fn slash_commands_are_left_alone() {
        assert_eq!(prompt_prefix_for("  /compact now", false), "  /compact now");
        let p = prompt_prefix_for("rename it", false);
        assert!(p.starts_with("<fridae_toolbox_instructions>\n## Fridae toolbox\n\n"), "{p}");
        assert!(p.ends_with("</fridae_toolbox_instructions>\n\n<user_request>\nrename it\n</user_request>"), "{p}");
    }
}
