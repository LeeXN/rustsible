#!/bin/bash
set -euo pipefail
for version in 1.0 2.0; do
    mkdir -p "/srv/versions/$version"
    for name in rustsible-fixture-one rustsible-fixture-two; do
        base=$(mktemp -d)
        mkdir -p "$base/usr/share/$name"
        printf '%s' "$version-1" > "$base/usr/share/$name/version"
        cat > "$base/.PKGINFO" <<EOF
pkgname = $name
pkgbase = $name
pkgver = $version-1
pkgdesc = Isolated Rustsible native fixture
url = https://example.invalid
builddate = 1700000000
packager = Rustsible tests
size = 8
arch = any
license = MIT
EOF
        (cd "$base" && bsdtar -czf "/srv/versions/$version/$name-$version-1-any.pkg.tar.gz" .PKGINFO usr)
        rm -rf "$base"
    done
    repo-add "/srv/versions/$version/fixture.db.tar.gz" /srv/versions/"$version"/*.pkg.tar.gz
    # libalpm compares repository modification times with second precision.
    # Distinct fixture revisions must not appear to share a publication time.
    touch -d "2020-01-0${version%%.*} UTC" "/srv/versions/$version/fixture.db.tar.gz"
done
