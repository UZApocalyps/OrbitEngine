OrbitEngine Editor: plugins shipped with the editor
===================================================

This folder sits next to the editor executable (installed or portable), as
`bundled-plugins/`. At start-up the editor installs the .orbitplugin packages
found here that were never installed before (same checks as an install from a
file), so the official plugins (Ren'Py import, MCP server and assistant) work
from the first launch. A plugin the user uninstalls or disables does not come
back. It may be empty: the editor then ships without plugins.

Fill it with `cargo xtask plugins --bundle` after building the editor in
release (default list: orbit.renpy_import and orbit.mcp; see DEFAULT_BUNDLED in
xtask/src/plugins.rs, or pass ids: `--bundle orbit.mcp,orbit.ai_studio`).
