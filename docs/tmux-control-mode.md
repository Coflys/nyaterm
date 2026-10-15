# tmux control mode

In a Desktop SSH terminal, run `tmux -CC` to project tmux windows and panes into
NyaTerm. Use `tmux -CC attach -t <session>` to attach to an existing session.
Single `-C` mode is also supported, including its terminal command echo.

## Local and remote splits

The tmux workspace occupies only the SSH leaf that started the control client.
Other local splits in the same tab remain visible and usable. Multiple SSH
leaves in one tab may each host a separate control client.

- The tmux strip inside that leaf switches remote windows. Window labels and
  order follow tmux window indexes, including `swap-window` and `move-window`.
- Split buttons in the tmux strip create remote tmux panes. Native tab split
  actions create local workspace splits around the selected host leaf.
- Local dividers remain draggable. Remote dividers keep the integration's
  existing fixed layout behavior; renderer dimensions are synchronized to tmux.
- Clicking a terminal selects the destination for AI and quick commands. A tmux
  child selects its host leaf without becoming a top-level tab.
- Focus mode first selects the local host leaf, then its selected tmux pane.

The window list refreshes on structural notifications and is checked every two
seconds for changes that emit no useful notification. Only one window-list
query may be outstanding at a time.

## Detach and recovery

Use the strip's detach button to leave the remote tmux session running and return
to the original SSH shell. The host terminal stays mounted through attach and
detach, preserving its scrollback; local layout and sibling terminals survive.
Closing a remote window with its strip's close button kills that tmux window.
Remote window tabs are runtime state and are not restored as native tabs.

History navigation only enables passive detection. A complete startup frame and
a matching nonce query response are required before NyaTerm creates virtual
sessions or sends initialization commands. Each confirmation stage has a fixed
five-second deadline and a 512 KiB buffer limit. Failure restores the original
SSH output path and replays buffered bytes, including incomplete trailing data.
