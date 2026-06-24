# OneNote Linux

A native Linux desktop application for viewing and editing imported Microsoft
OneNote notebooks.

The application uses the standalone
[`libonenote`](https://github.com/PatrykFlama/libonenote) package to import:

- `.onepkg` notebook exports
- `.one` section files
- `.onetoc2` notebook indexes and their adjacent sections

Imported notebooks open on a native, spatial page canvas. OneNote block positions
are preserved, complete ink stroke paths are rendered, and new freehand ink or
geometric shapes can be drawn directly over text and images. You can pan and zoom
pages, select/move/resize objects, partially erase or restyle ink, edit text and
table cells, and manage notebook sections and pages.

## File model

The Open dialog accepts every supported format: `.onl`, `.one`, `.onepkg`, and
`.onetoc2`. Opening `.one` or `.onepkg` makes that native file the active save
target. Opening `.onetoc2` imports the indexed notebook, but requires Save As
because a notebook index spans multiple files.

Edits are saved in an app-native `.onl` working copy with an adjacent
`<filename>.onl.assets` directory. Reopen the `.onl` file to continue editing.
The app can also export the notebook's editable content as Markdown.

For imported `.one` sections and `.onepkg` notebook exports, the app can also
save to the original native file or another file of the same native type when
changes map unambiguously to page titles or complete paragraphs and fit their
existing native property allocations. Save As can switch between the native
type and `.onl`; `.one`/`.onepkg` container conversion is not yet implemented.
Unsupported edits are rejected rather than omitted, with `.onl` offered as the
lossless fallback.

## OneDrive workflow direction

The project aims to support a normal cross-device workflow: open a notebook on
Linux, edit it, synchronize it, and see the changes in OneNote on another
device.

The implementation is split into two write paths:

1. **Microsoft Graph synchronization** for OneDrive and Microsoft 365
   notebooks. This is the first practical cross-device target. Graph supports
   signed-in access to notebooks, sections, and pages, and can create or patch
   page HTML.
2. **Native `.one` writing** for local notebooks and full-fidelity content.
   This requires a lossless revision-store writer that preserves unknown native
   records and updates only the objects changed by the editor.

`libonenote` now provides a loss-aware Microsoft Graph page serializer. Network
authentication, conflict detection, and synchronization UI live in this
application. Editable ink cannot currently be created through Graph, so native
writing remains necessary for a complete OneNote-compatible ink workflow.

Uploading reconstructed `.one` files directly into OneDrive notebook storage
is not treated as a synchronization mechanism; cloud notebooks will use the
supported OneNote API.

### Configure Microsoft sign-in

OneNote Linux uses OAuth device-code login and stores refresh tokens in the
desktop Secret Service keyring. The application needs a Microsoft Entra public
client registration:

1. Create an app registration that supports organizational directories and
   personal Microsoft accounts.
2. Under **Authentication**, enable public client flows.
3. Add the delegated Microsoft Graph permission `Notes.ReadWrite`.
4. Copy the application client ID into the **OneDrive…** dialog, or set:

```sh
export ONENOTE_LINUX_CLIENT_ID="<application-client-id>"
```

The client ID is saved in
`$XDG_CONFIG_HOME/onenote-linux/config.json` (normally
`~/.config/onenote-linux/config.json`). Access and refresh tokens are not stored
there or inside `.onl` files.

After a page has been uploaded, its Graph page ID and last-modified timestamp are
recorded in the `.onl` metadata. Later updates replace the app-managed page
content in place. Before writing, OneNote Linux checks the current remote
timestamp and blocks the update if the page changed in OneNote or on another
device.

Pages uploaded by versions that predate update support remain linked but cannot
be patched safely because they do not contain the app-managed content container.

## Run with Nix

From this repository:

```sh
nix run .
```

Open a OneNote export or an existing working copy directly:

```sh
nix run . -- ~/Downloads/Notebook.onepkg
nix run . -- ~/Documents/Notebook.onl
```

## Build with Cargo

Rust 1.92 or newer is required.

On Debian or Ubuntu, install the native dependencies:

```sh
sudo apt install build-essential pkg-config libgl1-mesa-dev libx11-dev \
  libxcursor-dev libxi-dev libxkbcommon-dev libxrandr-dev libwayland-dev
```

Then build and run:

```sh
cargo run --release -- ~/Downloads/Notebook.onepkg
```

Install the binary and desktop integration:

```sh
make install PREFIX="$HOME/.local"
update-desktop-database "$HOME/.local/share/applications"
update-mime-database "$HOME/.local/share/mime"
```

## Current capabilities

- Native X11 and Wayland desktop window
- Background import through `libonenote`
- Section and page navigation
- Pan-and-zoom spatial page canvas with OneNote coordinate import
- Editable notebook, section, and page names
- Positioned, movable, and resizable text, tables, images, and attachments
- Render complete imported ink paths with native color and width metadata
- Draw with smoothed pen and translucent highlighter tools
- Pressure-aware variable-width ink where the input backend exposes pressure,
  with speed-based dynamics as a Linux fallback
- Draw editable lines, arrows, rectangles, and ellipses; hold Shift to constrain
  their angle or proportions
- Pen palette, custom colors, width presets, and configurable eraser size
- Partially erase strokes without deleting the rest of the stroke
- Undo and redo drawing, erasing, deletion, and ink restyling
- Select, move, recolor, resize, and delete ink
- Add and delete sections, pages, text boxes, and tables
- Display imported images and save imported attachments
- Open `.onl`, `.one`, `.onepkg`, and `.onetoc2` from one file dialog
- Save and reopen `.onl` working copies
- Format-aware Save and Save As for `.onl`, `.one`, and `.onepkg`
- Save verified native `.one` and `.onepkg` copies for fixed-allocation text edits
- Export editable content to Markdown
- Sign in to Microsoft with device-code OAuth
- Discover OneDrive/Microsoft 365 OneNote notebooks and sections
- Manually upload a page and retain its Graph page link in the working copy
- Update linked pages in place with remote-change conflict detection

## Current limitations

- Native `.one` writing can shorten UTF-16-only text and replace text with the
  same encoded length. Growing text, resizing single-byte properties, layout,
  ink, structural changes, and merged paragraph boxes are not yet writable.
- `.onepkg` LZX input packages are rebuilt with MSZIP compression because the
  current CAB encoder cannot produce LZX. Package entries and OneNote structure
  are preserved and verified.
- Rich OneNote text styling is currently flattened to editable plain text.
- Linux window backends do not currently expose stylus pressure consistently, so
  dynamic pen width falls back to pointer speed when pressure data is unavailable.
- Handwriting recognition, automatic shape recognition, filled shapes, and lasso
  selection are not yet implemented.
- Graph uploads currently represent ink, images, attachments, and unknown native
  objects with explicit placeholders. Text and tables remain editable.
- Graph synchronization is manual and page-level; there is no background queue
  or live synchronization.
- A remote change blocks the local update. Automatic merge and a conflict
  resolution UI are not implemented yet.
- Pages uploaded before conflict-aware update support cannot be updated in
  place.
- Imported ink coordinates are reconstructed from OneNote's half-inch layout and
  native ink units; unusual files may need parser-specific calibration.
- Images and attachments are preserved but cannot yet be inserted or replaced.
- Password-protected or encrypted sections are unsupported.
- Very unusual or corrupt OneNote objects may be omitted during import.

Broader native OneNote writing remains in `libonenote`; the application exposes
each verified writer capability without duplicating revision-store logic.

For local development of both repositories, temporarily replace the pinned Git
dependency in `Cargo.toml` with `libonenote = { path = "../libonenote" }`.

## Development

```sh
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

### Source layout

```text
src/
├── app/
│   ├── mod.rs          # application state and UI panels
│   └── files.rs        # import, open, save, and export workflows
├── canvas/
│   ├── mod.rs          # canvas editor orchestration and tool UI
│   ├── drawing.rs      # stroke generation, smoothing, shapes, and erasing
│   ├── rendering.rs    # page, block, and ink rendering
│   └── history.rs      # undo/redo snapshots
├── graph/
│   ├── mod.rs          # OneDrive UI, background jobs, and sync state
│   └── client.rs       # OAuth, keyring, and Microsoft Graph HTTP client
├── project/
│   ├── mod.rs          # editable project model and persistence
│   ├── import.rs       # OneNote-to-editor model conversion
│   ├── native.rs       # verified editor-to-native edit mapping
│   └── graph.rs        # editor-page to Graph payload conversion
└── main.rs             # native application entry point
```

## License

OneNote Linux is licensed under the MIT License. `libonenote` and its parser
dependencies remain separately licensed packages.
