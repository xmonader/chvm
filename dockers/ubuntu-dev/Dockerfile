FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    curl \
    git \
    python3 \
    python3-pip \
    vim \
    net-tools \
    iproute2 \
    && rm -rf /var/lib/apt/lists/*

COPY demo.sh /usr/local/bin/demo.sh
RUN chmod +x /usr/local/bin/demo.sh

CMD ["/usr/local/bin/demo.sh"]
