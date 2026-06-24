PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share

.PHONY: build test install uninstall

build:
	cargo build --release

test:
	cargo test

install: build
	install -Dm755 target/release/onenote-linux "$(DESTDIR)$(BINDIR)/onenote-linux"
	install -Dm644 packaging/io.github.onenote-linux.Viewer.desktop "$(DESTDIR)$(DATADIR)/applications/io.github.onenote-linux.Viewer.desktop"
	install -Dm644 packaging/io.github.onenote-linux.Viewer.svg "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/io.github.onenote-linux.Viewer.svg"
	install -Dm644 packaging/io.github.onenote-linux.Viewer.xml "$(DESTDIR)$(DATADIR)/mime/packages/io.github.onenote-linux.Viewer.xml"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/onenote-linux"
	rm -f "$(DESTDIR)$(DATADIR)/applications/io.github.onenote-linux.Viewer.desktop"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/io.github.onenote-linux.Viewer.svg"
	rm -f "$(DESTDIR)$(DATADIR)/mime/packages/io.github.onenote-linux.Viewer.xml"
