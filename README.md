# apt.vulns.xyz

The source code for a reproducible apt repository.

## Dependencies for operating this repository

- Rust (unless you're using a pre-compiled binary)
- [repro-env](https://github.com/kpcyrd/repro-env) (depends on podman)
- git
- rsync

## Build a package

```
cargo run -- build <package>
```

A list of valid packages can be found in the `pkgs/` directory.

Built artifacts are available at:

```
./build/<package>/target/aarch64-unknown-linux-musl/debian/<package>_0.3.1~kpcyrd0_arm64.deb
./build/<package>/target/x86_64-unknown-linux-musl/debian/<package>_0.3.1~kpcyrd0_amd64.deb
```

## Adding .deb files to a reprepro repository

There's a reprepro configuration already setup in `conf/options`. After the `.deb` file is built you can add it to the package index with:

```
# This runs `reprepro includedeb stable ./build/.../<package>_0.3.1~kpcyrd0_arm64.deb` for all configured artifacts
# This can be dry-run with `-n`
cargo run -- include <package>
```

This needs access to a release signing key, if you are following along at home you need to edit `conf/options` to point to your own key.

To host your repository publicly, you need to upload `dists/` and `pool/` to a webserver.

## Reproducible Builds

The following packages have been fully integrated into the latest tooling:

- [x] acme-redirect
- [x] authoscope
- [ ] sh4d0wup - due to liblzma
- [ ] sn0int - due to ring 0.16.20 and libseccomp
- [ ] sniffglue - due to libpcap and libseccomp
- [x] spytrap-adb

They are expected to be bit-for-bit independently reproducible from source code, check the corresponding git tag.

Old versions and packages that don't build with the new tooling yet have been imported but can't be reproduced (without a significant amount of effort).

## Dependency tree vulnerability scanning

Packages listed above also have binaries with their resolved dependency tree embedded into them in the `.dep-v0` linker section. The binaries installed into the system can be scanned for known-vulnerable source-code inputs with:

```
cargo audit bin /usr/bin/<name>
```

The embedded json data can also be accessed directly, by zlib decompressing the section:

```
objcopy --dump-section .dep-v0=/dev/stdout /usr/bin/<name> | pigz -zd -
```

Feel free to open github issues in case there's anything needing attention.

## Configuration

At the time of writing, the typical build may look like this:

```toml
[meta]
repo = "https://github.com/..."
version = "0.3.1"
suffix = "~kpcyrd0"

[[checksums]]
path = "target/x86_64-unknown-linux-musl/debian/..._0.3.1~kpcyrd0_amd64.deb"
checksum = "sha256:..."

[[checksums]]
path = "target/aarch64-unknown-linux-musl/debian/..._0.3.1~kpcyrd0_arm64.deb"
checksum = "sha256:..."

[build]
cmd = """
set -e

# 2024-01-01
export SOURCE_DATE_EPOCH=1704067200

# configure the right linker for cross compile
mkdir -vp ~/.cargo
printf '[target.aarch64-unknown-linux-musl]\\nlinker = "/usr/aarch64-linux-musl/bin/musl-gcc"\\n' > ~/.cargo/config.toml

# select a specific Rust release so it's documented which one has been used
rustup default 1.80.1
rustup target add aarch64-unknown-linux-musl
rustup target add x86_64-unknown-linux-musl

cargo deb --locked --cargo-build 'auditable build' --deb-version "${DEB_VERSION}" --target aarch64-unknown-linux-musl
cargo deb --locked --cargo-build 'auditable build' --deb-version "${DEB_VERSION}" --target x86_64-unknown-linux-musl
"""
```

What this does (in order)

- Configure a git repository to clone from
- Configure a version string (`v${version}` is used as git tag to checkout)
- Configure a version suffix to ensure an official Debian package of that version would take precedence
- Configure the expected .deb build outputs and their expected checksum
- Configure a `SOURCE_DATE_EPOCH` for file timestamps used inside the .deb
- Configure the right linker so we can cross-compile Rust for aarch64
- Select a specific Rust release so it's documented which one has been used
- Download Rust musl toolchains for aarch64 and x86_64
- Build a statically linked release binary and embed the resolved dependency tree into a linker section for documentation purpose
- Use cargo-deb to bundle the binary into a .deb file

The build environment is documented in `pkgs/*/repro-env.toml` and `pkgs/*/repro-env.lock`.

## Updating the build environment

You can re-resolve the build environment using:

```
repro-env -C pkgs/<package> update
```

Note you may still need to bump versions referenced inline in `pkgs/<package>/build.toml`.

When doing this you likely also need to update the `[[checksums]]` rules.

## Trivia

This tooling replaces my old shell-scripted apt tooling that I started working on in May 2020 but never published.

## License

MIT / Apache-2.0
