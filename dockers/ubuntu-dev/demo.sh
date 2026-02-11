#!/bin/bash
echo "================================"
echo "  Ubuntu Dev Environment"
echo "================================"
echo ""

echo "--- System Info ---"
echo "OS       : $(cat /etc/os-release | grep PRETTY_NAME | cut -d= -f2 | tr -d '"')"
echo "Kernel   : $(uname -r)"
echo "Arch     : $(uname -m)"
echo ""

echo "--- Installed Tools ---"
echo "GCC      : $(gcc --version 2>/dev/null | head -1 || echo 'not found')"
echo "Make     : $(make --version 2>/dev/null | head -1 || echo 'not found')"
echo "Python3  : $(python3 --version 2>/dev/null || echo 'not found')"
echo "Git      : $(git --version 2>/dev/null || echo 'not found')"
echo "Curl     : $(curl --version 2>/dev/null | head -1 || echo 'not found')"
echo ""

echo "--- Compile & Run a C Program ---"
cat > /tmp/hello.c << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from a C program compiled inside a chvm microVM!\n");
    return 0;
}
EOF
gcc -o /tmp/hello /tmp/hello.c && /tmp/hello
rm -f /tmp/hello /tmp/hello.c
echo ""

echo "--- Run a Python Script ---"
python3 -c "
import platform, socket
print(f'Python {platform.python_version()} on {platform.system()} {platform.release()}')
print(f'Hostname: {socket.gethostname()}')
"
echo ""
echo "Done."
