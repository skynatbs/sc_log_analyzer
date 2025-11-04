# SC Log Analyzer

SC Log Analyzer is a tiny desktop viewer for Star Citizen `Game.log` files. It parses the log, groups the interesting player-related events, and presents them in a readable timeline that updates as the file changes.

## Quick Start
- Run the app. A native window will open.
- If the default path (`Game.log`) is not correct, paste the log path into the text box or press `Browse…` to pick it. The analyzer remembers the last file you opened.

## Reading the App
- **Event list**: The main panel shows the newest events first. Each type (kills, spawn loss, corpse state, zone moves, status effects, hits, vehicle destruction) gets its own color and short summary with extra details underneath.
- **Filters**: Use the checkboxes in the header to hide any event categories you do not care about.
- **Search**: The search box narrows the list to entries containing that text (it searches the summary, details, and original log line).
- **Ignore player**: Enter a handle to hide routine events triggered by that player. The app auto-fills this with the primary nickname found in the log unless you override it. Hit `Clear` to reset.
- **Player info**: Click a highlighted player name to fetch enlistment, location, fluency, and organization data from the RSI website. This needs an internet connection and may fail if the profile is private or missing.
- **Auto refresh**: The analyzer checks the selected file every couple of seconds and reloads automatically when it changes. Use `Reload` if you want to force a refresh immediately.

## Settings and Data
- Configuration files (last log path and ignored player) live in your user config directory, e.g. `%APPDATA%\sc_log_analyzer` on Windows or `~/.config/sc_log_analyzer` on Linux/macOS.
- No other data is stored. The tool only reads the log you point it at and the optional RSI profile pages you request.

That is all—open a log, tweak the filters, and scroll through the timeline.
