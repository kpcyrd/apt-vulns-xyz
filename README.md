# apt.vulns.xyz

The source code for a reproducible apt repository.

> [!NOTE]
> Versions from this repository haven't been deployed yet

## Dependencies for operating this repository

- Rust (unless you're using a pre-compiled binary)
- [repro-env](https://github.com/kpcyrd/repro-env) (depends on podman)
- git

## Build a package

```
cargo run -- <package>
```

A list of valid packages can be found in the `pkgs/` directory.

Built artifacts are available at:

```
./build/<package>/target/aarch64-unknown-linux-musl/debian/<package>_0.3.1~kpcyrd0_arm64.deb
./build/<package>/target/x86_64-unknown-linux-musl/debian/<package>_0.3.1~kpcyrd0_amd64.deb
```

## Reproducible builds

The following packages have been fully integrated into the latest tooling:

- [x] acme-redirect
- [x] authoscope
- [ ] sh4d0wup - due to liblzma
- [ ] sn0int - due to ring 0.16.20 and libseccomp
- [ ] sniffglue - due to libpcap and libseccomp
- [x] spytrap-adb

They are expected to be bit-for-bit independently reproducible from source code, check the corresponding git tag.

Old versions and packages that don't build with the new tooling yet have been imported but can't be reproduced (without a significant amount of effort).

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

wget https://www.musl-libc.org/releases/musl-1.2.5.tar.gz
echo 'a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4  musl-1.2.5.tar.gz' | sha256sum -c -
tar xf musl-1.2.5.tar.gz

pushd musl-1.2.5/
CROSS_COMPILE="aarch64-linux-gnu-" \
./configure --prefix=/usr/aarch64-linux-musl/lib/musl \
  --exec-prefix=/usr/aarch64-linux-musl \
  --enable-wrapper=all \
  --target="aarch64-linux-musl" \
  CFLAGS="-ffat-lto-objects"
make
make DESTDIR="/" install
mv -v /lib/ld-musl-aarch64.so* /usr/aarch64-linux-musl/lib/
popd

mkdir -vp ~/.cargo
printf '[target.aarch64-unknown-linux-musl]\\nlinker = "/usr/aarch64-linux-musl/bin/musl-gcc"\\n' > ~/.cargo/config.toml

rustup default 1.79.0
rustup target add aarch64-unknown-linux-musl
rustup target add x86_64-unknown-linux-musl
cargo deb --deb-version "${DEB_VERSION}" --target aarch64-unknown-linux-musl
cargo deb --deb-version "${DEB_VERSION}" --target x86_64-unknown-linux-musl
"""
```

What this does (in order)

- Configure a git repository to clone from
- Configure a version string (`v${version}` is used as git tag to checkout)
- Configure a version suffix to ensure an official Debian package of that version would take precedence
- Configure the expected .deb build outputs and their expected checksum
- Configure a `SOURCE_DATE_EPOCH` for file timestamps used inside the .deb
- Download and compile a specific musl libc version for aarch64 cross-compile
- Configure the linker we just built for Rust cross-compiling
- Select a specific Rust release
- Download musl toolchains for aarch64 and x86_64
- Run cargo-deb to build .deb files containing statically linked binaries

The build environment is documented in `pkgs/*/repro-env.toml` and `pkgs/*/repro-env.lock`.

## Trivia

This tooling replaces my old shell-scripted apt tooling that I started working on in May 2020 but never published.

## License

MIT / Apache-2.0
