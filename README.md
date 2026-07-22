# B.A.L.L.E.R.

**B**inary **A**llocation & **L**ibrary **L**aunch **E**nvironment in **R**ust

A cross-platform package manager for Linux & Windows — like Chocolatey, but with aviators.

---

## Installation

### From source

#### Linux

```bash
git clone https://github.com/HMythical/baller.git
cd baller

# Build a release binary
./build/linux/build.sh release

# Install to /usr/local/bin
sudo ./build/linux/build.sh install

# Uninstall
sudo ./build/linux/build.sh uninstall
```

#### Windows (PowerShell)

```powershell
git clone https://github.com/HMythical/baller.git
cd baller

# Build a release binary
.\build\winbuild\build.ps1 -Command release

# Install to %LOCALAPPDATA%\baller\bin
.\build\winbuild\install.ps1

# Uninstall
.\build\winbuild\uninstall.ps1
```

### From a release archive

1. Download the latest `baller-linux-*.tar.gz` or `baller-windows-*.zip` from the [Releases page](https://github.com/HMythical/baller/releases).
2. Extract the archive.
3. Move the `baller` (or `baller.exe`) binary to a directory on your `PATH`.

**Linux example:**

```bash
tar -xzf baller-linux-*.tar.gz
sudo mv baller /usr/local/bin/
```

**Windows example:**

```powershell
Expand-Archive baller-windows-*.zip -DestinationPath .
Move-Item .\baller.exe "$env:LOCALAPPDATA\baller\bin\baller.exe"
```

Verify the installation:

```bash
baller --version
```

---

## About

B.A.L.L.E.R is a general-purpose package manager for casual users and serious developers alike. Whether you want to install a JDK to play Minecraft or need specific library versions for a project, B.A.L.L.E.R's got your back.

The goal is to complement existing package managers (Chocolatey, apt, npm, NuGet, etc.) by providing access to both popular packages and more obscure ones that other managers don't distribute — all through a single, consistent CLI.

### Supported Sources

- **GitHub Releases** — download from GitHub release assets
- **Baller Registry** — custom registry API
- **Chocolatey/NuGet** — install from the Chocolatey community feed (SHA-512 verified)
- **System Package Manager** — wraps apt, dnf, or pacman for native Linux packages

---

## Building

### Linux

```bash
./build/linux/build.sh dev      # Debug build
./build/linux/build.sh release  # Release build (stripped)
./build/linux/build.sh test     # Run tests + clippy + fmt check
./build/linux/build.sh dist     # Create .tar.gz archive
./build/linux/build.sh clean    # Clean artifacts
```

### Windows (PowerShell)

```powershell
.\build\winbuild\build.ps1 -Command dev       # Debug build
.\build\winbuild\build.ps1 -Command release    # Release build
.\build\winbuild\build.ps1 -Command test       # Run tests
.\build\winbuild\build.ps1 -Command dist       # Create distribution packages
.\build\winbuild\build.ps1 -Command clean      # Clean artifacts
```

---

## Benchmarks

```bash
cargo bench
```

Reports timing for database operations, dependency resolution, download/extraction, and full package workflows.

---

## Contributing

We welcome contributions! Please read the [contributing guide](CONTRIBUTING.md) before submitting code. For security issues, contact us via Discord (HMythical).

---

## License

Apache 2.0. B.A.L.L.E.R will always be free and open-source.

---


Sincerely,
  HMythical
