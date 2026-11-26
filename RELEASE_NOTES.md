# Honeydew 0.0.15: pad rack fix, Clone (empty), DW Sampler browser

## Drum pads
- Adding an effect to a pad's FX rack now always goes into that pad's rack. Before, when the drum's main rack had no effects, the new effect landed in the drum's main rack instead.
- New `Clone (empty)` in a pad's menu, right under `Clone`: a copy of the pad (its sound, settings, FX rack and sends) with no hits, ready for a new pattern.

## DW Sampler
- Its browser now opens like the other modules' (3x Osc and the rest): at the sample or preset it plays, else at the root, instead of in `My Samples`.
