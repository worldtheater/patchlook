#!/bin/sh
set -eu
ssh-keygen -A
install -d -m 700 -o review -g review /home/review/.ssh
install -m 600 -o review -g review /run/test-key.pub /home/review/.ssh/authorized_keys
# Public fixture credential, valid only in this disposable loopback server.
printf 'review:patchlook-test\n' | chpasswd
runuser -u review -- python3 /opt/patchlook/seed.py
exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=no \
    -o PermitRootLogin=no -o AllowUsers=review -o AllowTcpForwarding=no -o X11Forwarding=no
