# Jumperless V5 docs

Markdown pages and example scripts copied from
https://github.com/Architeuthis-Flux/Jumperless-docs (`docs/`, commit 6517afd, 2026-09-30).
Public domain (see LICENSE). Images, 3D models and the built site were left out.

Start with `07.5-automation.md` (host control), `jumperless-agent-skill.md` and
`09.5-micropythonAPIreference.md`.

## Our board

Firmware 5.7.11.5 (checked 2026-10-27). On Linux the four USB serial ports are, by-id
`/dev/serial/by-id/usb-Architeuthis_Flux_Jumperless_V5_JLV5port-if0N`:

| by-id   | Port | Role                                                        |
|---------|------|-------------------------------------------------------------|
| `if00`  | 1    | Main menu terminal, `>` one-liner Python, single-char commands |
| `if02`  | 2    | Arduino UART passthrough                                    |
| `if04`  | 3    | MicroPython REPL (scripts)                                  |
| `if06`  | 4    | Read-only backchannel: `A` = JSON status, `:help` = YAML     |

Use the by-id names, never `/dev/ttyACMx`: the ACM numbers shift, and one of them is
not a Jumperless.
