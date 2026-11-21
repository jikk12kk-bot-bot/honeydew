Honeydew 0.0.10: the grid is part of the theme, and the analyzer options move.

- Theme creator. The grid's look is now set in the theme, not in Settings:
  - `Grid`: the colour of the vertical beat and bar lines in the playlist and the editors (piano roll, drum grid, automation, audio editor).
  - `Horizontal grid` (new): the colour of the lines between rows in the playlist, the piano roll and the drum grid, and of the automation editor's value lines.
  - `Grid` group, `Thickness` and `Horizontal thickness`: how thick the vertical and the horizontal lines are, from ×0 (hidden) to ×4, in steps of ×0.25.
  - The preview at the top shows the grid as you edit it. Saved, exported and imported themes keep it.
- Settings. `Grid color:` is gone from SETTINGS. A grid colour picked there before becomes the theme's `Grid` colour the first time this version opens.
- Analyzer options.
  - Settings: `Limit analyzer refresh` is now `Analyzer refresh limit:`, a value of its own: Off, or 15, 24, 30, 45, 60 (the default), 75, 90, 120, 144 or 240 updates a second. It sets how often the module displays update at most (the Analyzer, the Tuner, the Limiter and every other module that draws its sound); fewer updates take less CPU from the audio. It changes only the pictures, never the sound.
  - The Analyzer's SETTINGS page: `Pause hidden` and `Light pitch` (formerly in Settings) now sit here, on a third row. They are global: switching one in any Analyzer switches it for all of them, and the app remembers them; songs do not save them.
