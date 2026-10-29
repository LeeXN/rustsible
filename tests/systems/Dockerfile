FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends systemd systemd-sysv dbus openssh-server sudo ca-certificates libcap2-bin \
    && rm -rf /var/lib/apt/lists/* /etc/ssh/ssh_host_* \
    && mkdir -p /run/sshd /srv/rustsible /srv/http \
    && useradd -m -s /bin/sh deploy \
    && useradd -m -s /bin/sh app \
    && printf 'deploy ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/deploy \
    && chmod 0440 /etc/sudoers.d/deploy \
    && systemctl enable ssh \
    && systemctl mask systemd-logind.service getty.target console-getty.service
STOPSIGNAL SIGRTMIN+3
CMD ["sh", "-c", "ssh-keygen -A >/dev/null && exec /sbin/init"]
