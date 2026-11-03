FROM scratch
ADD rootfs.tar.xz /
RUN mkdir -p /etc/xbps.d && printf 'repository=https://repo-de.voidlinux.org/current\n' > /etc/xbps.d/00-repository-main.conf && xbps-install -Syu xbps && xbps-install -Syu && xbps-install -y rust cargo gcc python3 shadow sudo coreutils procps-ng libxkbcommon-tools dbus runit xorg-server-xvfb xauth xkbcomp kmod eudev shellcheck
COPY . /source
RUN useradd -m -u 1987 numberpadqa && chown -R numberpadqa /source
USER numberpadqa
ENV CARGO_HOME=/tmp/numberpad-cargo CARGO_TARGET_DIR=/tmp/numberpad-target
WORKDIR /source/numberpad
RUN cargo fetch --locked && cargo test --locked --all-features && python3 dependency-inventory.py --check && cargo build --locked --release && NUMBERPAD_BINARY=/tmp/numberpad-target/release/asus-numberpad ./install /tmp/numberpad-production-stage && /tmp/numberpad-production-stage/usr/bin/asus-numberpad build-info | grep '"test_fixtures":false' && cargo build --locked --release --features test-fixtures
USER root
WORKDIR /source
RUN install -m755 /tmp/numberpad-target/release/asus-numberpad /usr/bin/asus-numberpad && shellcheck numberpad/install numberpad/test-native.sh && sh -n numberpad/test-native.sh
CMD ["sh", "/source/numberpad/test-native.sh", "void"]
