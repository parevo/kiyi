# An SSH bastion for testing tunnels: the databases are reachable from here as
# `postgres:5432` and `mysql:3306`, like a jump host in front of a private subnet.
FROM alpine:3.20
RUN apk add --no-cache openssh \
 && ssh-keygen -A \
 && adduser -D -s /bin/sh kiyi \
 && echo "kiyi:kiyi" | chpasswd \
 && mkdir -p /home/kiyi/.ssh \
 && sed -i -e 's/^#*AllowTcpForwarding.*/AllowTcpForwarding yes/' \
           -e 's/^#*PasswordAuthentication.*/PasswordAuthentication yes/' \
           -e 's/^#*StrictModes.*/StrictModes no/' /etc/ssh/sshd_config
CMD ["/usr/sbin/sshd", "-D", "-e"]
