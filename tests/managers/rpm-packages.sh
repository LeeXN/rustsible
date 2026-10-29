#!/bin/sh
set -eu
mkdir -p /srv/rpmbuild/BUILD /srv/rpmbuild/BUILDROOT /srv/rpmbuild/RPMS /srv/rpmbuild/SOURCES /srv/rpmbuild/SPECS /srv/rpmbuild/SRPMS
for version in 1.0 2.0; do
    mkdir -p "/srv/versions/$version"
    for name in rustsible-fixture-one rustsible-fixture-two; do
        cat > /srv/rpmbuild/SPECS/fixture.spec <<EOF
Name: $name
Version: $version
Release: 1
Summary: Native package acceptance fixture
License: MIT
BuildArch: noarch
%description
An isolated version marker used only by Rustsible tests.
%install
mkdir -p %{buildroot}/usr/share/$name
printf '%s' '$version-1' > %{buildroot}/usr/share/$name/version
%files
/usr/share/$name/version
EOF
        rpmbuild --define '_topdir /srv/rpmbuild' --define '_binary_payload w9.gzdio' -bb /srv/rpmbuild/SPECS/fixture.spec
        cp "/srv/rpmbuild/RPMS/noarch/$name-$version-1.noarch.rpm" "/srv/versions/$version/"
    done
    # Yum also uses repository publication time when comparing revisions.
    stamp=$((1577836800 + ${version%%.*} * 86400))
    createrepo_c --no-database --compress-type gz --revision "$stamp" --set-timestamp-to-revision "/srv/versions/$version"
    touch -d "@$stamp" "/srv/versions/$version/repodata/repomd.xml"
done
