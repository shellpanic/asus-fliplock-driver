FROM lazyvoid-qa:workspace
RUN xbps-install -Sy python3-evdev python3-gobject libxkbcommon-tools elogind dbus runit && xbps-query -p pkgver python3-evdev && xkbcli dump-keymap-wayland --help && xkbcli dump-keymap-x11 --help
COPY . /source
WORKDIR /source
ENV PYTHONPATH=/source/numberpad
RUN python3 -m unittest discover -s numberpad/tests -v && numberpad/install /stage && /stage/usr/bin/asus-numberpad --help
CMD ["python3", "-m", "unittest", "discover", "-s", "numberpad/tests", "-v"]
