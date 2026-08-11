# PBI Lens

A fast, local-first Power BI report explorer for macOS. PBI Lens opens `.pbix` and `.pbit` packages without Windows or uploading report data.

## Install

On an Apple Silicon Mac:

```sh
npx @seg4lt/pbi-lens
```

The installer downloads the latest GitHub release, installs the app, and removes the macOS quarantine flag. PBI Lens checks for a signed update at most once per day; when one is available, a small toolbar icon lets you choose whether to install and restart.

```sh
npx @seg4lt/pbi-lens update     # download and install the newest release
npx @seg4lt/pbi-lens launch     # open the installed app
npx @seg4lt/pbi-lens uninstall  # remove the installed app
```

## What it explores

- Report pages, visual placement, types, titles, and bound fields
- PBIX and PBIT semantic-model tables, raw imported rows, column statistics, measures, calculated columns, relationships, partitions, security metadata, and DAX expressions
- Full Power Query M definitions with line numbers, search, connector/source discovery, dependencies, and copy actions
- Paginated local VertiPaq table browsing and CSV page copy
- Visual selection with geometry, layer, field bindings, and metadata inspection
- Every file inside the Power BI package with compressed/original sizes plus text or hex inspection
- Drag-and-drop, native Open dialog, Finder file associations, and recent files

## Development

```sh
npm install
npm run tauri dev
```

Build the macOS application:

```sh
npm run tauri build -- --bundles app
```

Local builds use the separate `PBI Lens Dev` app name and `com.pbilens.desktop.dev` bundle ID, and do not check for production updates. GitHub Actions merges `src-tauri/tauri.prod.conf.json` to build the release as `PBI Lens` with bundle ID `com.pbilens.desktop`.

## Architecture

The UI is Svelte 5 and plain CSS. The Tauri backend is Rust and parses the ZIP-based Power BI container locally with `zip` and `serde_json`. A bundled native arm64 helper decodes VertiPaq semantic models locally; it is kept warm while the app runs so table paging is immediate. Large parsing work runs off the UI thread. The app does not require an account, server, or analytics SDK; its only network feature is the signed GitHub update check.

PBI Lens never invents report values. Report pages are reconstructed from packaged layout metadata, while model values shown in Data are decoded from the file's imported VertiPaq tables. Live-connection reports may contain metadata without imported rows.

Power BI and related trademarks belong to Microsoft Corporation. PBI Lens is an independent project and is not affiliated with Microsoft.
