---
name: roblox-playtest
description: Test Roblox UI and game behavior in a live Studio playtest using the Roblox Studio MCP tools. Use when verifying UI changes, clicking buttons, typing, triggering prompts, inspecting runtime state, or checking the console after a change.
---

# Roblox playtest (Roblox Studio MCP)

Runs a real playtest in Studio and drives it like a player. Everything here
uses the `mcp__Roblox_Studio__*` tools. If they are deferred, load them in one
ToolSearch call:

`select:mcp__Roblox_Studio__list_roblox_studios,mcp__Roblox_Studio__start_stop_play,mcp__Roblox_Studio__execute_luau,mcp__Roblox_Studio__user_mouse_input,mcp__Roblox_Studio__user_keyboard_input,mcp__Roblox_Studio__screen_capture,mcp__Roblox_Studio__get_console_output,mcp__Roblox_Studio__character_navigation,mcp__Roblox_Studio__get_studio_state`

## Tools

| Job | Tool |
|---|---|
| Pick the Studio window (`studio_id`) | `list_roblox_studios` |
| Start / stop the playtest | `start_stop_play` (`is_start` true / false) |
| Run Luau on the client | `execute_luau` with `datamodel_type: "Client"` |
| Run Luau on the server | `execute_luau` with `datamodel_type: "Server"` |
| Click, hover, scroll | `user_mouse_input` |
| Press keys, type text | `user_keyboard_input` |
| Walk the character somewhere | `character_navigation` |
| See the screen | `screen_capture` (works during play; omit the camera arguments) |
| Read the Output log | `get_console_output` |

`execute_luau` returns whatever the code `return`s, so return a string that
states the result instead of printing.

## Workflow

1. `list_roblox_studios`, take the `id`. If several places are open, ask which.
2. `start_stop_play` with `is_start: true`.
3. Run the steps: input tools to act, `execute_luau` to check state,
   `screen_capture` when the look matters.
4. `start_stop_play` with `is_start: false`. **Always**, whether the test
   passed, failed or got blocked. A playtest left running blocks editing.
5. `get_console_output` and report any errors or warnings the game produced.

Reset player data, if a test needs a fresh player, in edit mode before step 2.

## Clicking

Click by coordinates taken from the live button. Many games have unnamed or
duplicate-named buttons (`Frame.Frame.TextButton`), so `instance_path` is
unreliable.

Get the centre on the client:

```lua
local b = game.Players.LocalPlayer.PlayerGui.MainGui.HUD.ShopButton
local c = b.AbsolutePosition + b.AbsoluteSize / 2
return string.format("%d,%d", c.X, c.Y)
```

Then `user_mouse_input` with
`[{action:"moveTo", x, y}, {action:"mouseButtonClick", mouse_button:"left"}, {action:"wait", wait_time_ms:300}]`.

- Pass `AbsolutePosition` values as they are. Do **not** add the top-bar inset
  (`GuiService:GetGuiInset()`), even for `IgnoreGuiInset` ScreenGuis; adding it
  lands the click about 58 px too low.
- Coordinates are viewport pixels, not screenshot pixels. Screenshots are
  scaled down, so never read click positions off an image.
- When buttons share a name, find the right one by its text or a child
  label's text, or by position, before taking its centre.
- Scroll with `moveTo` over the ScrollingFrame, then `scrollDown` / `scrollUp`.

## Keyboard

`user_keyboard_input`: `keyPress` with a `key_code` (`"Return"`, `"Tab"`,
`"E"`), `keyDown` / `keyUp` for holds, `textInput` with `text_inputs` to type.
To type into a TextBox, click it first so it has focus.

## Proximity prompts

Walk there with `character_navigation` (`instance_path` of the part or model),
then hold the prompt's key: `keyDown`, `wait` for `HoldDuration` plus a little,
`keyUp`. Read `KeyboardKeyCode` and `HoldDuration` from the prompt first.

## Test like a player

Use simulated input. Never fake it with `button.Activated:Fire()`,
`prompt.Triggered:Fire()`, controller calls such as `ShopController:OpenShop()`,
or by calling reward functions directly. Never force state with
`Visible = true` or similar to get past a step; that hides the bug being tested.

Reading state to verify a result is fine and expected:

```lua
local f = game.Players.LocalPlayer.PlayerGui.MainGui.ShopFrame
return "ShopFrame.Visible=" .. tostring(f.Visible)
```

Studio syncs client and server almost instantly, so skip long waits. A short
`wait` step (200-500 ms) after a click for a tween is enough.

## When the game misbehaves

If something unexpected happens (element hidden, path missing, wrong state),
do not work around it. Take one `screen_capture`, stop the playtest, read the
console, and report:

```
BLOCKED: [what went wrong]
Expected: [what should have happened]
Tried: [steps run]
Console: [relevant errors]
```

Then investigate from the source code in edit mode, not by poking the running
game. Don't retry the same failing step more than twice.

## Notes

- Luau runs on Studio's main thread. Keep test scripts short; any waiting loop
  must call `task.wait()`.
- Listeners left on `shared` or connected during a test die with the playtest,
  but disconnect ones you no longer need in long sessions.
- DataStore warnings such as `StudioAccessToApisNotAllowed` come from the
  place's settings, not from the test.
