---
name: ui-design
description: "Building and checking Roblox UI. Build or restyle Roblox UI (HUDs, menus, shops, popups, buttons), matching references and the game genre across devices. Use before creating, laying out or restyling any GUI."
examples:
  - "Build a HUD with coins and a settings button"
  - "Restyle the shop menu to match this screenshot"
  - "Make the inventory work on phones and consoles"
  - "Check my menus for overlapping or cut-off UI"
applies: game
core: true
---

# Roblox UI design

Uses the Roblox Studio MCP tools (see roblox-studio-workflow). Two files sit
beside this one: `helpers.lua` (builder functions) and `audit.lua` (defect
check). Read them when needed and paste their contents into `execute_luau`.

For polish, motion and style direction, general design guides are installed
beside this one (`emil-design-eng`, `motion-design`, `redesign-skill`,
`reference-design-contract`, `soft-skill`, `minimalist-skill`, `brutalist-skill`). They are written for the
web: read `web-to-roblox.md` in this folder for which to use and how their
CSS maps to Roblox.

## First, look at what exists

- Live Studio is the truth, not source files. Find the UI with
  `search_game_tree` (`path: "StarterGui"`, `keywords` for the menu name), then
  read its hierarchy and properties with `inspect_instance`. Never guess a path.
- Many games build UI from scripts at runtime, so StarterGui can be nearly
  empty. Then read the scripts that build it, and inspect the live result in a
  playtest (`search_game_tree` with `datamodel_type: "Client"`).
- Reuse what the project has: templates cloned at runtime, UI helper modules,
  a theme module, image ids, and the UIStroke/UICorner values other screens
  use. When editing or extending existing UI, match its style exactly.
- Build new static UI as instances under StarterGui with `execute_luau` (Edit),
  and keep scripts for behaviour.

## Taste (read before styling anything)

UI must look designed and restrained: clean, tasteful and quiet, not the generic Roblox look. This is the default
for every game unless the user's house rules, their own designs or their request for this game say otherwise.

- **Their designs win.** Illustrator or Photoshop exports, mockups, a style sheet, colour values or fonts from the user
  are the target, ahead of every guide and every genre habit. Build to them exactly; ask for exported slices (PNG or
  SVG), the fonts and the colour values instead of inventing art or "improving" their choices.
- **Palette:** neutrals plus one or two accents, taken from the game's art direction (PROJECT.md). Colour marks meaning
  (the primary action, a state), not decoration. No rainbows, no candy colours, no colour per button.
- **Typography does the work:** one or two typefaces, a clear size scale, real hierarchy, generous spacing, everything
  aligned to a grid. Leave space empty rather than filling it.
- **Few effects:** thin outlines or none, flat or very subtle fills, no thick black strokes, no glossy bevels, no
  gradients unless the direction calls for one, shadows only to separate layers. Motion is short and quiet.
- **Fewer things:** no stacked "NEW!" badges, bursts, sparkles or shine sweeps, no decoration that carries no meaning.
- **The cartoon simulator look** (chunky outlined bubble buttons, rainbow gradients, stacked strokes, cartoon fonts)
  is used only when the user asks for it for this game; then read `roblox-sim-ui`.
- Before calling UI done, ask: would a careful designer sign this? If it looks like a template from the Toolbox, it
  is not done.

## Style target and scope

- A supplied reference image is the style target. Match its proportions,
  silhouette, density, typography, surfaces, gradients and icon treatment.
  Look at the actual image. Before building, note approximate normalized
  x/y/width/height for its landmarks (window, header, content grid, detail
  panel, HUD groups) and lay out from those numbers; keep the note in the
  builder so later repairs use the same target.
- Without a reference, extend the game's existing style. A new game with no
  style yet gets the clean default under Taste, whatever its genre. Read
  `roblox-sim-ui` only when the user asks for the cartoon simulator look.
- UI requests change UI and the local behaviour it needs. Don't add a lobby,
  terrain, lighting, models or economy systems to dress up a screenshot.
- Before layout, find the game's asset pack and image ids and reuse them
  across screens. Keep one theme table and one icon table per game.

## Building

Start a builder with the contents of `helpers.lua`, set its `THEME` to the
game's palette, then compose:

```lua
local gui = UI.screen("Shop", 10)
local win = UI.window(gui, "Shop", { Size = UDim2.fromScale(0.5, 0.6), Aspect = 1.5 })
UI.grid(win.Body, UDim2.fromScale(0.3, 0.45))
local card = UI.panel(win.Body, { Name = "SwordCard", Color = UI.Theme.Secondary })
UI.art(card, ICONS.Sword, { Size = UDim2.fromScale(0.6, 0.45), Position = UDim2.fromScale(0.2, 0.08) })
UI.text(card, "Sword", { Size = UDim2.fromScale(0.9, 0.18), Position = UDim2.fromScale(0.05, 0.52) })
UI.button(card, "BUY", UI.Theme.Primary, { Size = UDim2.fromScale(0.8, 0.22), Position = UDim2.fromScale(0.5, 0.84) })
```

- `UI.screen(name, displayOrder)` makes (or replaces) a ScreenGui in StarterGui.
- `UI.window(gui, title, {Size, Aspect, Position})` returns
  `{Root, Header, Body, Close}`.
- `UI.panel`, `UI.text`, `UI.art`, `UI.button(parent, text, color, props)`,
  `UI.grid`, `UI.list`, `UI.round`, `UI.outline`, and `UI.new(class, props, parent)`
  for anything else.

The helpers are a starting kit, not a required silhouette. When a reference
calls for a different shell, build it from `UI.new` and primitives. Windows
start `Visible = true` for screenshots; set the closed state before finishing.

Layout and hierarchy:

- Match the reference's columns, rows and occupied screen area. Choose tile
  density from content and viewport: a collection index may want a dense 4x3
  grid, a pass shop larger illustrated cards.
- Use role colours consistently (buy/claim/play, secondary, danger, selected,
  locked). Glossy gradients only when the user's target has them.
- Short words: "BUY", "CLAIM", "EQUIP"; a 2-4 word tagline at most.
- HUD: keep the centre of the screen clear.
- Main menu: big title, one dominant PLAY, smaller secondary buttons, the 3D
  world visible behind it.

## Icons and images

Icons make the most difference. Use real images, never emoji or invented ids.
The texture generator is not for UI art, icons or anything with lettering,
so in order:

1. The project's own image ids, or images the user supplies.
2. Items that exist as models: render them with the icon booth in one
   consistent style and upload (roblox-studio-workflow, "Icons and renders").
3. Creator Store images: `search_asset` with `assetType: "Image"`.
4. Settings, arrows and close glyphs: native text or simple shapes.

If an illustration is still missing, leave a labelled slot and tell the user
which art is missing. Don't pass a letter badge off as final art. Keep ids in
one `ICONS` table. Look at every image before uploading (moderation rules are
in roblox-studio-workflow). New uploads can show blank for a few minutes.

## Surface rules (what audit.lua checks)

Roblox's defaults are what make UI look unfinished: a new Frame, TextLabel or
ImageLabel has an opaque grey or white fill, a border and the legacy font.

- Never leave an engine default. Every GuiObject sets `BackgroundTransparency`
  (1 for text and art), a theme `BackgroundColor3` when it has a fill,
  `BorderSizePixel = 0`, the theme font, `TextColor3` and real `Text`.
- Every image: no fill, no border, `ScaleType.Fit`, aspect locked, and no
  UICorner/UIStroke/UIGradient on the image itself. Don't sit art on a light
  plate.
- Every button reads as a button: a fill, an outline or button art, in the
  game's direction (by default a flat or subtle fill, small corner, thin or no
  outline; gradients and raised lips only when the direction has them). Never
  text floating on nothing.
- Text in buttons and cards: `TextScaled` with a `UITextSizeConstraint`
  (min 9-12) so it shrinks instead of clipping, about 8% side padding, inside
  its parent's bounds. Size the button for its longest label.
- Panels and HUD cards: opaque or near-opaque theme colour with an outline.
- A big window body may be slightly see-through: a dark or saturated colour
  at `BackgroundTransparency` 0.2-0.35. Its outline stays opaque, and so does
  everything on it. Never a translucent white or light fill; it reads as a
  grey box.

## Layout essentials

- Scale for panels and regions, offset for strokes and padding. Keep shapes
  with `UIAspectRatioConstraint`. Centre with `AnchorPoint (0.5, 0.5)` at
  `UDim2.fromScale(0.5, 0.5)`; pin to corners with `(1, 0)`, `(0, 1)` etc.
- A layout object positions its parent's direct GuiObject children and
  ignores their `Position`. Keep decorations, headers and backgrounds outside
  the layout container. Scale `CellSize` cells stay square only with a
  `UIAspectRatioConstraint` in each cell.
- ScrollingFrame: `AutomaticCanvasSize` on the scroll axis, `CanvasSize =
  UDim2.new()`, layout inside, padding so the bar doesn't cover content.
- One ScreenGui per layer (HUD, Shop, Popups), `ResetOnSpawn = false`, ordered
  with `DisplayOrder`. Open and close by toggling `Visible` or `Enabled`.

## Input, devices, motion

- Buttons fire on `Activated` (mouse, touch, gamepad). Touch targets at least
  44 px on a phone. Leave the bottom-left thumbstick and bottom-right jump
  areas clear. Keep `ScreenInsets = CoreUISafeInsets` for anything clickable.
- Press feedback: tween the button's `ButtonScale` UIScale (hover about 1.05,
  press about 0.95). Stronger bounce for toy styles, restrained for sleek or
  horror. Update UI from events or data observers, not every frame.

## Validate, screenshot, repair

1. **Audit.** Run `audit.lua` through `execute_luau`: Edit datamodel checks
   StarterGui, Client datamodel in a playtest checks PlayerGui. Fix every
   `default_look`, `white_box`, `art_background`, `art_decorated`,
   `text_clipped`, `text_outside`, `no_text_limit`, `canvasgroup_text` and
   `small_target`, then run it again. Then check the hierarchy: count each
   grid's direct GuiObject children against the intended items (a stray
   decorative Frame eats a cell), and check rows, columns, padding and scroll
   extent.
2. **Screenshot.** `screen_capture` with no camera arguments shows the UI over
   the current view. Look at the image. A script that ran without error is not
   visual validation.
3. **Compare** with the reference: window size and position, header height,
   rows and columns, surface opacity, outline weight, font scale, spacing.
   Without one, compare against the chosen theme. Name the largest mismatch.
4. **Repair** that region in the existing code and capture again. Keep correct
   regions and asset ids; don't rebuild the whole screen to fix one card.
   Repeat until nothing material is off, or report the exact remaining gap.
5. **Devices and behaviour.** Check a phone-sized layout, then playtest
   opening, closing, scrolling and buttons with real clicks (roblox-playtest).
   Stop every playtest. Report any check that could not be run as unverified.
