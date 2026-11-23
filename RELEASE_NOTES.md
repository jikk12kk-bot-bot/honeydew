# Honeydew 0.0.14: an FX rack for every drum pad, pads you can reorder, Combine for drum clips

## Drum pads
- Every drum pad has its own FX rack. In the drum editor, select a pad's row (Kick, Snare, …) and the rack on the right shows that pad's rack, with its name in the header (`Drum · Kick`). Its audio effects work on that pad's sound only, and its note effects (Randomizer, Arpeggiator, …) work on that pad's hits only.
- The sound goes pad → pad rack → drum rack → mixer. Select the Drum track in the playlist (outside the drum editor) to see the drum rack for the whole drum, as before.
- Pad racks are saved in the song. Deleting a pad also removes its rack. FL Studio Mobile plays a pad rack's effects in the drum rack instead.
- To reorder pads, long press a pad's name and drag it up or down; a line shows where it will land. Its hits in every drum clip move with it, so the beat sounds the same. Long press and let go without moving to open the pad's menu, as before.
- The `AUX` button is gone from the drum strip's `Seq` page. Pads already routed to an FX channel stay routed.

## Combine
- `Combine` works on drum clips too.
- Combined clips keep every cut. A note or held hit that ran past the end of a cut clip now ends at the cut, as you heard it, instead of running on through the cut part. The combined clip starts where the first clip starts.

## Track headers
- With the mixer column open, long press a track's icon and let go to open the track menu, or long press and drag to move the track, the same as when the column is closed.

## Lemon Studio plugins
- The built-in Lemon plugins are updated to Lemon Studio's latest (`349c6ba`). Several Lemon Samplers playing the same sample share one copy of it in memory, and a sample path can name environment variables (`%NAME%`). On the desktop, text in a plugin's editor that its own font can't draw now uses a system font that can.
