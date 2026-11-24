# Honeydew 0.0.14: an FX rack and sends for every drum pad, pads you can reorder, Combine for drum clips

## Drum pads
- Every drum pad has its own FX rack. In the drum editor, select a pad's row (Kick, Snare, …) and the rack on the right shows that pad's rack, with its name in the header (`Drum · Kick`). Its audio effects work on that pad's sound only, and its note effects (Randomizer, Arpeggiator, …) work on that pad's hits only.
- The sound goes pad → pad rack → drum rack → mixer. Select the Drum track in the playlist (outside the drum editor) to see the drum rack for the whole drum, as before.
- Pad racks are saved in the song. Deleting a pad also removes its rack. FL Studio Mobile plays a pad rack's effects in the drum rack instead.
- To reorder pads, long press a pad's name and drag it up or down; a line shows where it will land. Its hits in every drum clip move with it, so the beat sounds the same. Long press and let go without moving to open the pad's menu, as before.
- The `AUX` button is gone from the drum strip's `Seq` page. Pads already routed to an FX channel stay routed.
- Every drum pad has its own SENDS, audio and MIDI, like a track's. Select a pad's row and the rack's SENDS shows them. The first row is where the pad's sound goes (its drum track, or an FX channel it was routed to with the old `AUX`), and you can change it. An extra audio send adds the pad's sound, after its own rack, to another track beside its main output. A MIDI send plays the pad's hits on another track as that pad's note.
- `Clone` on a pad also copies its FX rack.

## Combine
- `Combine` works on drum clips too.
- Combined clips keep every cut. A note or held hit that ran past the end of a cut clip now ends at the cut, as you heard it, instead of running on through the cut part. The combined clip starts where the first clip starts.

## Lemon Studio plugins
- The built-in Lemon plugins are updated to Lemon Studio's latest (`349c6ba`). Several Lemon Samplers playing the same sample share one copy of it in memory, and a sample path can name environment variables (`%NAME%`). On the desktop, text in a plugin's editor that its own font can't draw now uses a system font that can.

## Controls
- Every knob and slider (mixer, mini mixer, drum strips, rack, effect panels) starts dragging the moment you touch it, even if you hold still first. Its menu opens only when you long press and let go without moving. A right click opens the menu at once.
- Reverb 2's DELAY shows beats (`0.50 beat`) when Tempo is on, instead of milliseconds.

## Plugins
- An edit in a plugin's window (a knob, a curve, a sample loaded, a menu pick) is now an undo step and marks the song as unsaved.
