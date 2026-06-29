# duhem

The command-line interface for [Duhem](https://github.com/onsager-ai/duhem) —
a holistic verification platform for AI-delivered software.

## Install

```bash
npm install -g duhem
# or run without installing
npx duhem --help
```

This package is a thin launcher. The real binary ships per-platform via an
optional dependency (`@duhem/cli-<os>-<arch>`); npm installs only the one
matching your machine. Prebuilt targets: linux-x64, linux-arm64, darwin-x64,
darwin-arm64, windows-x64.

Prebuilt binaries are also attached to each
[GitHub Release](https://github.com/onsager-ai/duhem/releases).

## Usage

```bash
duhem init        # scaffold a Verification Definition
duhem validate    # validate VDs / manifest against the schema
duhem run         # run a verification and emit a verdict
duhem dashboard   # serve / export the run dashboard
duhem --version
```

See the [project README](https://github.com/onsager-ai/duhem#readme) for the
full documentation.

## License

Apache-2.0
