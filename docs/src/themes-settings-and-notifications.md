# Themes, Settings, and Notifications

## Themes

Arbor includes a large theme set and a theme picker modal.

Keyboard interaction now includes:

- arrow-key movement in the theme grid
- `Enter` to apply
- `Escape` to dismiss
- visible selected theme state

## Settings

The settings surface includes:

- daemon bind mode
- notifications toggle
- GitHub auth-related state and connected daemon behavior

## Notifications

Arbor supports both local and remote notification paths:

- native desktop notifications from the GUI
- webhook delivery from the daemon

The repo config can filter notifications by event name. Current webhook event names are:

- `agent_started`
- `agent_finished`
- `agent_error`

Webhook delivery is transition-aware and retrying:

- repeated `working -> working` or `waiting -> waiting` updates do not emit duplicate webhook events
- agent activity emits `agent_started` when a session moves into working state and `agent_finished` when it moves into waiting state
- transient webhook failures are retried with a short bounded backoff
- Slack incoming webhooks receive a `text` payload and Discord webhooks receive a `content` payload

## Command Palette UX

The command palette is designed to support keyboard-first navigation:

- `Cmd+K` opens it
- arrow keys move selection
- the list auto-scrolls to keep the selected item visible
- the mouse only changes selection after actual mouse movement
- the overflow indicator and count show when more results exist
