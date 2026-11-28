//! Who may do what (port of T3 Code's McpToolAccess.ts, threadAccess.ts and delegatedTaskModes.ts).
//!
//! Fridae's modes on T3's ranks: a chat whose last message ran in Full auto or Studio is `full-access` (3), any other
//! `approval-required` (0); Autopilot and Overnight add no rank. Fridae has no plan mode, so interaction is always
//! `default`. Modes are read live from the chat on every call, never from the key.

use std::sync::Arc;

use super::{Call, Failure, card, fail, keys};
use crate::agents;

/// A chat's runtime mode on T3's scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Runtime {
    ApprovalRequired,
    FullAccess,
}

impl Runtime {
    pub fn rank(self) -> u8 {
        match self {
            Runtime::ApprovalRequired => 0,
            Runtime::FullAccess => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::ApprovalRequired => "approval-required",
            Runtime::FullAccess => "full-access",
        }
    }
}

/// A chat's interaction mode: Fridae has only T3's `default`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interaction {
    Default,
}

impl Interaction {
    pub fn rank(self) -> u8 {
        1
    }

    pub fn as_str(self) -> &'static str {
        "default"
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub runtime: Runtime,
    pub interaction: Interaction,
}

/// The chat a call speaks for, with its modes as they are now.
pub struct Caller {
    pub agent: Arc<agents::Agent>,
    pub scope: keys::Scope,
    pub limits: Limits,
}

/// A chat's modes, from its last message's switches.
pub fn limits_of(agent: &agents::Agent) -> Limits {
    let ask = agent.last_ask();
    Limits { runtime: if ask.full || ask.studio { Runtime::FullAccess } else { Runtime::ApprovalRequired }, interaction: Interaction::Default }
}

/// The calling chat, for a call that only reads.
pub fn read_caller(scope: &keys::Scope) -> Result<Caller, Failure> {
    let agent = agents::get(&scope.chat).ok_or_else(|| fail("thread_not_found", "The calling chat was not found."))?;
    let limits = limits_of(&agent);
    Ok(Caller { agent, scope: scope.clone(), limits })
}

/// The calling chat, for a call that changes something: it must be live (not closed, on the key's AI, a turn
/// running), so a key cannot act for a chat after its run.
pub fn writing_caller(scope: &keys::Scope) -> Result<Caller, Failure> {
    let caller = read_caller(scope)?;
    let a = &caller.agent;
    if a.is_closed() || a.ai() != scope.ai || !a.is_working() {
        return Err(fail("parent_not_active", "The calling AI no longer owns an active chat run."));
    }
    Ok(caller)
}

/// A chat this call changes must not run with broader modes than the caller's.
pub fn target_within(caller: &Caller, target: &agents::Agent) -> Result<(), Failure> {
    let t = limits_of(target);
    let c = caller.limits;
    if t.runtime.rank() > c.runtime.rank() || t.interaction.rank() > c.interaction.rank() {
        return Err(fail(
            "runtime_mode_escalation_denied",
            format!(
                "Chat \"{}\" runs in {}/{}, above this caller's {}/{} limit. This call does not create an approval request. Have the user act directly in the target chat.",
                target.id,
                t.runtime.as_str(),
                t.interaction.as_str(),
                c.runtime.as_str(),
                c.interaction.as_str()
            ),
        ));
    }
    Ok(())
}

/// The runtime mode of a chat the caller starts: the caller's when omitted or "inherit", lower when asked, never higher.
pub fn resolve_runtime(parent: Runtime, requested: Option<&str>) -> Result<Runtime, Failure> {
    let resolved = match requested.map(str::trim) {
        None | Some("inherit") => parent,
        Some("approval-required") => Runtime::ApprovalRequired,
        Some("full-access") => Runtime::FullAccess,
        Some(v) => return Err(fail("invalid_request", format!("Unknown runtime mode \"{v}\". Use approval-required, full-access or inherit."))),
    };
    if resolved.rank() > parent.rank() {
        return Err(fail("runtime_mode_escalation_denied", format!("Child runtime mode {} is broader than parent mode {}.", resolved.as_str(), parent.as_str())));
    }
    Ok(resolved)
}

/// The interaction mode of a chat the caller starts (always `default` in Fridae).
pub fn resolve_interaction(parent: Interaction, requested: Option<&str>) -> Result<Interaction, Failure> {
    let resolved = match requested.map(str::trim) {
        None | Some("inherit") | Some("default") => Interaction::Default,
        Some(v) => return Err(fail("invalid_request", format!("Unknown interaction mode \"{v}\". Use default or inherit."))),
    };
    if resolved.rank() > parent.rank() {
        return Err(fail("interaction_mode_escalation_denied", format!("Child interaction mode {} is broader than parent mode {}.", resolved.as_str(), parent.as_str())));
    }
    Ok(resolved)
}

/// A change to projects, settings or the look (T3's writesEnvironment, with Fridae's card where T3 refuses): in Full
/// auto it just happens; below it, and always when `always` (deletes, guarded settings, adding an existing folder,
/// anything that publishes or spends), the user is asked with a card in the calling chat and the call waits.
pub fn environment_gate(call: &Call, caller: &Caller, card: card::CardSpec, always: bool) -> Result<(), Failure> {
    if !always && caller.limits.runtime == Runtime::FullAccess {
        return Ok(());
    }
    match card::failure_of(&card::ask(&caller.agent, &caller.scope.ai, card, call.cancelled)) {
        Some(f) => Err(f),
        None => Ok(()),
    }
}

/// The open chat a call names: the caller itself when no id is given and the tool defaults to this chat (T3 does
/// for update, organize, configure and queue), else `target_required`.
pub fn resolve_chat(caller: &Caller, given: Option<&str>, default_this: bool) -> Result<Arc<agents::Agent>, Failure> {
    match given.filter(|g| !g.trim().is_empty()) {
        None if default_this => Ok(caller.agent.clone()),
        None => Err(fail("target_required", "Pass chatId.")),
        Some(id) => agents::get(id).ok_or_else(|| chat_not_found(id)),
    }
}

/// The chat id in a `fridae://chat/<id>` link, if `given` holds one.
fn linked_id(given: &str) -> Option<String> {
    let at = given.find("fridae://chat/")? + "fridae://chat/".len();
    let id: String = given[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!id.is_empty()).then_some(id)
}

/// A miss, with the right id when the given one is a chat link or a near spelling of an open or saved chat's id
/// (whitespace or backticks around it, or a missing or extra `c`).
pub fn chat_not_found(given: &str) -> Failure {
    if let Some(id) = linked_id(given) {
        return fail("thread_not_found", format!("`{given}` is a chat link, not a chat id. Pass `{id}` instead."));
    }
    let norm = given.trim().trim_matches('`').trim();
    let bare = |s: &str| s.strip_prefix('c').unwrap_or(s).to_string();
    let mut ids: Vec<String> = agents::agents().iter().map(|a| a.id.clone()).collect();
    ids.extend(agents::history().iter().filter_map(|c| c["id"].as_str().map(str::to_string)));
    match ids.into_iter().find(|id| !norm.is_empty() && (id == norm || bare(id) == bare(norm))) {
        Some(id) => fail("thread_not_found", format!("The chat was not found. Did you mean `{id}`?")),
        None => fail("thread_not_found", "The chat was not found."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Ask;
    use crate::app_mcp::exposure::Mode;
    use crate::app_mcp::{drop_chat, test_chat};
    use std::sync::atomic::AtomicBool;

    fn scope_of(agent: &agents::Agent, ai: &str) -> keys::Scope {
        keys::Scope { chat: agent.id.clone(), ai: ai.to_string(), subagent: false, namespace: "ns".into() }
    }

    fn full() -> Ask {
        Ask { full: true, ..Default::default() }
    }

    #[test]
    fn ranks_match_t3() {
        assert_eq!((Runtime::FullAccess.rank(), Runtime::ApprovalRequired.rank()), (3, 0));
        assert_eq!((Runtime::FullAccess.as_str(), Runtime::ApprovalRequired.as_str()), ("full-access", "approval-required"));
        assert_eq!(Interaction::Default.as_str(), "default");
    }

    #[test]
    fn limits_come_from_the_last_message() {
        let a = test_chat("claude");
        assert_eq!(limits_of(&a).runtime, Runtime::ApprovalRequired);
        a.set_last_ask_for_test(full());
        assert_eq!(limits_of(&a).runtime, Runtime::FullAccess);
        a.set_last_ask_for_test(Ask { studio: true, ..Default::default() });
        assert_eq!(limits_of(&a).runtime, Runtime::FullAccess, "Studio runs in full auto");
        a.set_last_ask_for_test(Ask { autopilot: true, overnight: true, ..Default::default() });
        assert_eq!(limits_of(&a).runtime, Runtime::ApprovalRequired, "autopilot and overnight add no rank");
        drop_chat(&a);
    }

    #[test]
    fn resolve_runtime_inherits_and_never_widens() {
        assert_eq!(resolve_runtime(Runtime::FullAccess, None), Ok(Runtime::FullAccess));
        assert_eq!(resolve_runtime(Runtime::FullAccess, Some("inherit")), Ok(Runtime::FullAccess));
        assert_eq!(resolve_runtime(Runtime::FullAccess, Some("approval-required")), Ok(Runtime::ApprovalRequired));
        assert_eq!(resolve_runtime(Runtime::ApprovalRequired, None), Ok(Runtime::ApprovalRequired));
        let e = resolve_runtime(Runtime::ApprovalRequired, Some("full-access")).unwrap_err();
        assert_eq!(e.code, "runtime_mode_escalation_denied");
        assert_eq!(e.message, "Child runtime mode full-access is broader than parent mode approval-required.");
        let e = resolve_runtime(Runtime::FullAccess, Some("auto")).unwrap_err();
        assert_eq!((e.code, e.message.as_str()), ("invalid_request", "Unknown runtime mode \"auto\". Use approval-required, full-access or inherit."));
        assert_eq!(resolve_interaction(Interaction::Default, None), Ok(Interaction::Default));
        assert_eq!(resolve_interaction(Interaction::Default, Some("inherit")), Ok(Interaction::Default));
        assert_eq!(resolve_interaction(Interaction::Default, Some("plan")).unwrap_err().code, "invalid_request");
    }

    #[test]
    fn a_target_above_the_caller_is_refused_with_t3s_words() {
        let me = test_chat("claude");
        let other = test_chat("claude");
        other.set_last_ask_for_test(full());
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        let e = target_within(&caller, &other).unwrap_err();
        assert_eq!(e.code, "runtime_mode_escalation_denied");
        assert_eq!(
            e.message,
            format!("Chat \"{}\" runs in full-access/default, above this caller's approval-required/default limit. This call does not create an approval request. Have the user act directly in the target chat.", other.id)
        );
        // the same or lower is fine
        me.set_last_ask_for_test(full());
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        assert!(target_within(&caller, &other).is_ok());
        drop_chat(&me);
        drop_chat(&other);
    }

    #[test]
    fn writes_need_a_live_caller() {
        let me = test_chat("claude");
        let scope = scope_of(&me, "claude");
        let e = writing_caller(&scope).err().unwrap();
        assert_eq!((e.code, e.message.as_str()), ("parent_not_active", "The calling AI no longer owns an active chat run."));
        me.set_running_for_test(true);
        assert!(writing_caller(&scope).is_ok());
        assert_eq!(writing_caller(&scope_of(&me, "gemini")).err().unwrap().code, "parent_not_active", "a key for another AI");
        me.set_running_for_test(false);
        drop_chat(&me);
        let e = writing_caller(&scope).err().unwrap();
        assert_eq!((e.code, e.message.as_str()), ("thread_not_found", "The calling chat was not found."));
    }

    #[test]
    fn reads_need_no_live_caller() {
        let me = test_chat("claude");
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        assert_eq!(caller.agent.id, me.id);
        assert_eq!(caller.limits.runtime, Runtime::ApprovalRequired);
        drop_chat(&me);
    }

    #[test]
    fn chat_miss_suggests_the_id() {
        let me = test_chat("claude");
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        let e = resolve_chat(&caller, Some("c-no-such-chat-1"), false).err().unwrap();
        assert_eq!((e.code, e.message.as_str()), ("thread_not_found", "The chat was not found."));
        let quoted = format!("`{}` ", me.id);
        let e = resolve_chat(&caller, Some(&quoted), false).err().unwrap();
        assert_eq!(e.message, format!("The chat was not found. Did you mean `{}`?", me.id));
        let bare = me.id.trim_start_matches('c').to_string();
        assert_eq!(resolve_chat(&caller, Some(&bare), false).err().unwrap().message, format!("The chat was not found. Did you mean `{}`?", me.id));
        assert_eq!(resolve_chat(&caller, Some(&me.id), false).ok().map(|a| a.id.clone()), Some(me.id.clone()));
        drop_chat(&me);
    }

    #[test]
    fn a_chat_link_is_not_an_id() {
        let me = test_chat("claude");
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        for given in [format!("fridae://chat/{}", me.id), format!("[Mine](fridae://chat/{})", me.id)] {
            let e = resolve_chat(&caller, Some(&given), false).err().unwrap();
            assert_eq!(e.code, "thread_not_found");
            assert_eq!(e.message, format!("`{given}` is a chat link, not a chat id. Pass `{}` instead.", me.id));
        }
        drop_chat(&me);
    }

    fn gate_card() -> card::CardSpec {
        card::CardSpec { title: "Change a setting".into(), text: "Turn on \"Install updates by itself\".".into(), allow: "Turn it on".into() }
    }

    /// `environment_gate` on another thread, as a tool's handler runs it.
    fn gating(me: &Arc<agents::Agent>, always: bool) -> std::thread::JoinHandle<Result<(), Failure>> {
        let me = me.clone();
        std::thread::spawn(move || {
            let scope = scope_of(&me, "claude");
            let caller = writing_caller(&scope)?;
            let flag = AtomicBool::new(false);
            let call = Call::new(&scope, "call-1", &flag, Mode::Full);
            environment_gate(&call, &caller, gate_card(), always)
        })
    }

    #[test]
    fn environment_gate_full_auto_needs_no_card() {
        let me = test_chat("claude");
        me.set_last_ask_for_test(full());
        me.set_running_for_test(true);
        assert_eq!(gating(&me, false).join().unwrap(), Ok(()));
        assert!(!me.log_snapshot().iter().any(|e| e["kind"] == "appcard"));
        drop_chat(&me);
    }

    #[test]
    fn environment_gate_below_full_auto_asks() {
        let me = test_chat("claude");
        me.set_running_for_test(true);
        let h = gating(&me, false);
        let id = card::tests::wait_event(&me, "appcard", None)["card"].as_str().unwrap().to_string();
        assert!(card::answer(&me.id, &id, true));
        assert_eq!(h.join().unwrap(), Ok(()));
        let h = gating(&me, false);
        let first = id;
        let id = loop {
            let e = card::tests::wait_event(&me, "appcard", None);
            if e["card"] != first.as_str() {
                break e["card"].as_str().unwrap().to_string();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(card::answer(&me.id, &id, false));
        assert_eq!(h.join().unwrap().unwrap_err().code, "declined");
        drop_chat(&me);
    }

    #[test]
    fn always_asks_even_in_full_auto() {
        let me = test_chat("claude");
        me.set_last_ask_for_test(full());
        me.set_running_for_test(true);
        let h = gating(&me, true);
        let id = card::tests::wait_event(&me, "appcard", None)["card"].as_str().unwrap().to_string();
        assert!(card::answer(&me.id, &id, true));
        assert_eq!(h.join().unwrap(), Ok(()));
        drop_chat(&me);
    }

    #[test]
    fn default_this_only_when_asked() {
        let me = test_chat("claude");
        let caller = read_caller(&scope_of(&me, "claude")).unwrap();
        assert_eq!(resolve_chat(&caller, None, true).ok().map(|a| a.id.clone()), Some(me.id.clone()));
        let e = resolve_chat(&caller, None, false).err().unwrap();
        assert_eq!((e.code, e.message.as_str()), ("target_required", "Pass chatId."));
        drop_chat(&me);
    }
}
