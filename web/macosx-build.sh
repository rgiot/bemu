export CC_wasm32_unknown_unknown=$(brew --prefix llvm)/bin/clang
export AR_wasm32_unknown_unknown=$(brew --prefix llvm)/bin/llvm-ar
"$(dirname "$0")/build.sh"
