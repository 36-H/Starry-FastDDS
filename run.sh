#!/bin/bash
./build_img.sh riscv64
make A=apps/monolithic_userboot ARCH=riscv64 run LOG=error