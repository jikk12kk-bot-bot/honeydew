# Honeydew 0.0.11: values you can read, and the grid everywhere

## Values
- The floating value bubble is gone, and so is the `Show value while dragging` setting. The mixer no longer turns a channel's name into its level while you drag the fader.
- Values are shown in fixed places instead, like the Analyzer's `440 Hz`:
  - Mixer: the level in dB under each fader is larger (8 pt), and it stays visible when the strip gets short, for example after moving the keyboard up. The pan knob shows its value on its ring.
  - Playlist mini mixer: the pan and level show above their sliders (and an automation lane's slider too).
  - Drum strips: Level, Pan, Hold and the other sliders show their value above the thumb; the envelope knobs show theirs on the ring.
  - A send's level in the rack shows above its slider.

## Grid
- The horizontal grid is as thick as the vertical grid's beat lines at the same thickness (both ×4 now look the same).
- The piano roll's horizontal lines are no longer faded.
- The drum step sequencer shows the theme's grid: step, beat and bar lines and the lines between the rows.
- Module displays use the theme's grid colours and thickness: the Analyzer (every view, with semitone lines on PITCH), Spectral Ceiling, Limiter and Multiband (now with vertical lines too), Autoduck, Tuner, Note Echo, and a grid on Equalizer, Dynamic EQ, Tremolo, Pitcher, Wow & Flutter, the Slicer waveform and the 3x Osc LFO and envelope graphs.

## Analyzer and settings
- The Analyzer's `Pause hidden` and `Light pitch` switches move to their own `GLOBAL` tab, each with a description of what it does. The SETTINGS page is back to its two rows, so nothing overlaps.
- Settings `Analyzer refresh limit` is a slider in steps of 1: `Off` (no limit), 1 … 240 per second. The default stays 60.
