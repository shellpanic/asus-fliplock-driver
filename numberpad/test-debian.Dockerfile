FROM rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55
RUN apt-get update && apt-get install -y --no-install-recommends python3 procps libxkbcommon-tools xkb-data dbus systemd runit sudo shellcheck kmod udev xvfb xauth x11-xkb-utils && rm -rf /var/lib/apt/lists/*
COPY . /source
RUN useradd -m -u 1987 numberpadqa && chown -R numberpadqa /source
USER numberpadqa
ENV CARGO_HOME=/tmp/numberpad-cargo CARGO_TARGET_DIR=/tmp/numberpad-target
WORKDIR /source/numberpad
RUN cargo fetch --locked && cargo test --locked --all-features && python3 dependency-inventory.py --check && cargo build --locked --release && NUMBERPAD_BINARY=/tmp/numberpad-target/release/asus-numberpad ./install /tmp/numberpad-production-stage && /tmp/numberpad-production-stage/usr/bin/asus-numberpad build-info | grep '"test_fixtures":false' && cargo build --locked --release --features test-fixtures
USER root
WORKDIR /source
RUN install -m755 /tmp/numberpad-target/release/asus-numberpad /usr/bin/asus-numberpad && shellcheck numberpad/install numberpad/test-native.sh && sh -n numberpad/test-native.sh
CMD ["sh", "/source/numberpad/test-native.sh", "debian"]
