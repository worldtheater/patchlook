FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends openssh-server git python3 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/sh review && passwd -d review \
    && mkdir -p /run/sshd /home/review/.ssh \
    && chmod 700 /home/review/.ssh
COPY seed.py /opt/patchlook/seed.py
COPY start.sh /opt/patchlook/start.sh
EXPOSE 22
CMD ["sh", "/opt/patchlook/start.sh"]
