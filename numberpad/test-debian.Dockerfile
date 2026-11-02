FROM debian:12-slim
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-evdev python3-gi libxkbcommon-tools xkb-data dbus systemd runit && rm -rf /var/lib/apt/lists/*
COPY . /source
WORKDIR /source
ENV PYTHONPATH=/source/numberpad
RUN python3 -m unittest discover -s numberpad/tests -v && numberpad/install /stage && /stage/usr/bin/asus-numberpad --help
CMD ["python3", "-m", "unittest", "discover", "-s", "numberpad/tests", "-v"]
