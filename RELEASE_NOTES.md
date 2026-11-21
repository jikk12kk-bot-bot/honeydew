Honeydew 0.0.10: the grid is part of the theme.

- Theme creator. The grid's look is now set in the theme, not in Settings:
  - `Grid`: the colour of the vertical beat and bar lines in the playlist and the editors (piano roll, drum grid, automation, audio editor).
  - `Horizontal grid` (new): the colour of the lines between rows in the playlist, the piano roll and the drum grid, and of the automation editor's value lines.
  - `Grid` group, `Thickness` and `Horizontal thickness`: how thick the vertical and the horizontal lines are, from ×0 (hidden) to ×4, in steps of ×0.25.
  - The preview at the top shows the grid as you edit it. Saved, exported and imported themes keep it.
- Settings. `Grid color:` is gone from SETTINGS. A grid colour picked there before becomes the theme's `Grid` colour the first time this version opens.
- Piano roll. The lines between rows are drawn like the playlist's: 1 px of the theme's horizontal grid colour (`background` by default).
