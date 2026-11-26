# Honeydew 0.0.15: pinned and recent modules, pad rack fix, Clone (empty), DW Sampler browser

## Modules browser
- Pin the modules you use most: long press a module (let go without moving) or right-click it, then pick `Pin`. Pinned modules show in a PINNED section at the top, under `Paste`, `Rack Preset` and `Instrument`, in the order you pinned them. Do the same again and pick `Unpin` to remove one.
- A RECENT section under PINNED shows the last 5 modules you added, newest first. Pinned modules are left out of it, and it updates the next time you open the browser.
- Both sections show in Category and A-Z order and while searching, with only the modules the channel can take. In A-Z the full list follows under ALL. The modules still appear in their usual sections too, and your pins and recent modules are kept across restarts.

## Drum pads
- Adding an effect to a pad's FX rack now always goes into that pad's rack. Before, when the drum's main rack had no effects, the new effect landed in the drum's main rack instead.
- New `Clone (empty)` in a pad's menu, right under `Clone`: a copy of the pad (its sound, settings, FX rack and sends) with no hits, ready for a new pattern.

## DW Sampler
- Its browser now opens like the other modules' (3x Osc and the rest): at the sample or preset it plays, else at the root, instead of in `My Samples`.
