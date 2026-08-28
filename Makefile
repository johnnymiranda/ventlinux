# VentLinux install rules.
#
#   make                      build release binaries
#   sudo make install         install to /usr/local
#   make install PREFIX=~/.local   install for the current user only
#   sudo make uninstall
#
# Packagers: `make DESTDIR="$pkgdir" PREFIX=/usr install`. When DESTDIR is set
# the desktop database is left alone -- that is the package manager's job.

PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

APPID := com.cryptexlabs.ventlinux
# Honour CARGO_TARGET_DIR if the caller exports one, so `make install` looks
# where `make build` actually put the binaries.
RELEASE := $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target)/release

BINDIR := $(DESTDIR)$(PREFIX)/bin
APPDIR := $(DESTDIR)$(PREFIX)/share/applications
LICDIR := $(DESTDIR)$(PREFIX)/share/licenses/ventlinux

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --locked

# Deliberately does not depend on `build`: packagers build once in build() and
# install in package(), and a rebuild there would run outside the build sandbox.
install:
	@test -x $(RELEASE)/ventlinux || { echo "error: $(RELEASE)/ventlinux missing -- run 'make build' first" >&2; exit 1; }
	install -Dm755 $(RELEASE)/ventlinux $(BINDIR)/ventlinux
	install -Dm755 $(RELEASE)/ventctl   $(BINDIR)/ventctl
	install -Dm644 crates/ventlinux/$(APPID).desktop $(APPDIR)/$(APPID).desktop
	install -Dm644 LICENSE $(LICDIR)/LICENSE
	install -Dm644 vendor/libventrilo3/COPYING $(LICDIR)/COPYING.libventrilo3
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null 2>&1; then \
		update-desktop-database $(APPDIR) || true; \
	fi

uninstall:
	rm -f $(BINDIR)/ventlinux
	rm -f $(BINDIR)/ventctl
	rm -f $(APPDIR)/$(APPID).desktop
	rm -rf $(LICDIR)
	@if [ -z "$(DESTDIR)" ] && command -v update-desktop-database >/dev/null 2>&1; then \
		update-desktop-database $(APPDIR) || true; \
	fi

clean:
	$(CARGO) clean
