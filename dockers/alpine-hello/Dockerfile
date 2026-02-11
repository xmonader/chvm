FROM alpine:latest

RUN apk add --no-cache curl bash

COPY hello.sh /usr/local/bin/hello.sh
RUN chmod +x /usr/local/bin/hello.sh

CMD ["/usr/local/bin/hello.sh"]
