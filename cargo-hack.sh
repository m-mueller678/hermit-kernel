set -e
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net --features pci
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features gem-net,rtl8139 --features tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features gem-net,rtl8139 --features pci,tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features gem-net,virtio-net --features tcp,rtl8139
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target x86_64-unknown-none --exclude-features gem-net,virtio-net --features pci,tcp,rtl8139
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64-unknown-none-softfloat --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64-unknown-none-softfloat --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net --features pci
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64-unknown-none-softfloat --exclude-features gem-net,rtl8139 --features tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64-unknown-none-softfloat --exclude-features gem-net,rtl8139 --features pci,tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64_be-unknown-none-softfloat -Zbuild-std=core,alloc --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64_be-unknown-none-softfloat -Zbuild-std=core,alloc --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net --features pci
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64_be-unknown-none-softfloat -Zbuild-std=core,alloc --exclude-features gem-net,rtl8139 --features tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target aarch64_be-unknown-none-softfloat -Zbuild-std=core,alloc --exclude-features gem-net,rtl8139 --features pci,tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features dhcpv4,dns,gem-net,net,rtl8139,virtio-net --features pci
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features gem-net,rtl8139 --features tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features gem-net,rtl8139 --features pci,tcp,virtio-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features rtl8139,virtio-net --features tcp,gem-net
cargo hack check --package hermit-kernel --each-feature --no-dev-deps --target riscv64gc-unknown-none-elf --exclude-features rtl8139,virtio-net --features pci,tcp,gem-net
