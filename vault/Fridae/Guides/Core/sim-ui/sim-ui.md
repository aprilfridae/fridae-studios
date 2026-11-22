---
name: roblox-sim-ui
description: The chunky cartoon simulator look (Shop, Daily Rewards, Rebirth, Codes, Settings, HUD buttons). Use ONLY when the user asks for this cartoon simulator look for the game; the default for every genre is the clean look in roblox-ui-design.
---

# Simulator UI

A look spec, built with the helpers in roblox-ui-design. The visual language
follows Check-UI Clean Roblox by diggofun / TemporaryLooker22 (MIT; see
LICENSE.md in this folder). No module is installed for you: build these
values from primitives, or use the Check-UI modules if the user has added
them to the place (check with `search_game_tree`, `keywords: "CheckUI"`).

Use with roblox-ui-design. This is the default look for a new simulator,
pet-collection, tycoon or cartoon game. A supplied reference image or the
game's existing style wins over it. Don't apply it to a horror or sci-fi game.

## Theme

Set the helper THEME to this before building:

```lua
THEME.Panel = Color3.fromRGB(59, 88, 102)      -- slate canvas #3B5866
THEME.Outline = Color3.fromRGB(24, 30, 34)     -- #181E22
THEME.Font = Enum.Font.GothamBlack
THEME.TitleFont = Enum.Font.GothamBlack
THEME.Radius = 4
THEME.Edge = 3.5
THEME.WindowAlpha = 0.3
```

## The look

- **Window:** slate canvas at BackgroundTransparency 0.3 so the world shows
  through the body slightly, 4 px corners, 3.5 px dark outline that stays
  opaque. Only the big window body is see-through. Header, cards, tiles,
  wells, buttons, price pills, inputs and text stay opaque.
- **Header:** a saturated two-colour vertical gradient per menu (Shop magenta
  to crimson, Codes cyan, Rebirth purple, Daily lime, Settings silver), a 2 px
  light line on top, a two-tier divider below (3 px dark accent over 3 px
  black), a left-aligned 38 px title, and a square red close button on the
  right.
- **Type:** GothamBlack everywhere, white, with a black UIStroke of 2-3.5 px.
  Titles 38, buttons 22-26, card titles 22-24, small text 13-15.
- **Cards:** bold diagonal gradient, 2.8 px black outline, a thin pastel
  inner rim, and a slowly rotating sunburst behind the art. Recessed wells
  (#1A2C34) hold lists and inputs.
- **Buttons:** gradient face, a 3 px darker bevel along the bottom, a pastel
  inner rim, outlined label. State colours: Buy/Claim lime, Claimed/disabled
  silver, Locked red, Skip gold, feature pink, Off grey. Prices sit in a lime
  button with the Robux icon.
- **Motion:** every clickable thing is centre-anchored with a `ButtonScale`
  UIScale: hover 1.06, press 0.94, Back-out release. Windows pop in from 0.72
  scale. Sunbursts spin about 20 degrees a second while the window is visible.
- **Responsive:** one UIScale per window = viewport / (1050 x 620), clamped
  0.52 (phones) to 1.18 (large desktop).

## Menus

Design sizes in pixels, scaled by the window's UIScale:

| Menu | Size | Contents |
|---|---|---|
| Shop | 600 x 415 | Sections of cards: 2 per row for passes, 3 for products; each card has art, title, one-line description and a price button |
| Daily Rewards | 640 x 440 | 7 day cards with Claimed / Claim / Locked states; day 7 is a tall rainbow card |
| Rebirth | 528 x 430 | From and To cards with an arrow between, a requirement line, a progress bar with percent text, a Rebirth button |
| Codes | 528 x 262 | A hint line, a TextBox in a recessed well, a Redeem button |
| Settings | 528 wide | Rows of title, subtitle and an on/off toggle button |

HUD buttons go on the left edge as square buttons with an icon and a one-word
label inside. Keep the play area clear.

One LocalScript per menu handles open/close, the pop-in tween, button press
feedback and the sunburst spin, and stops the spin when the window closes.

## Icons and art

1. The game's own image ids first.
2. Creator Store images (`search_asset`, `assetType: "Image"`), choosing a set
   in one style: glossy chunky cartoon, thick dark outline, saturated colours.
3. Icon-booth renders of the game's own models (pets, eggs, chests); see
   roblox-studio-workflow, "Icons and renders".
4. If nothing fits, leave the slot with an outlined glyph ("$", "x2", "?")
   instead of a blank box, and tell the user which art is missing.

Never invent an asset id. The texture generator is not for UI art or icons.

## Rules

- Run `audit.lua` (roblox-ui-design) before every screenshot and fix what it
  lists: default fills and fonts, light boxes, clipped or escaping text, art
  with backgrounds.
- Labels stay inside their button or card. Size buttons for their longest
  label; keep labels to one or two words.
- Custom panels get the same see-through body in a dark or saturated colour.
  Never make a white or light fill see-through, and never a card, button,
  price pill, well or text background.

## Finish

Follow roblox-ui-design: audit, screenshot, compare with the reference or this
look, repair the biggest mismatch, re-shoot, check a phone-sized viewport,
then playtest open/close and buy buttons with real clicks.
