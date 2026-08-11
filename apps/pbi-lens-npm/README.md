# @seg4lt/pbi-lens

Install or update the latest PBI Lens release on an Apple Silicon Mac:

```sh
npx @seg4lt/pbi-lens
```

The installer downloads the latest official GitHub release, copies PBI Lens to `/Applications` (or your user Applications folder), recursively removes the `com.apple.quarantine` flag, and launches it. The desktop app then checks for signed updates at most once per day and installs only when you choose **Update and restart**.

```sh
npx @seg4lt/pbi-lens update
npx @seg4lt/pbi-lens launch
npx @seg4lt/pbi-lens uninstall
```
