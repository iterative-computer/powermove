//! The lookup table of the ported LLVM libc `atan`.
//!
//! Copied verbatim (hexadecimal float literals unchanged) from llvm-project
//! `libc/src/__support/math/atan_utils.h` at revision 3ea89f4304312567e31a8eb45e0737577e65b676, the
//! revision Chromium 152 (Electron 44's V8 15.2) pins.
//!
//! Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use super::dd::DoubleDouble;
use super::hexf::hexf;

pub(crate) const ATAN_I: [DoubleDouble; 65] = [
    DoubleDouble {
        lo: hexf("0.0"),
        hi: hexf("0.0"),
    },
    DoubleDouble {
        lo: hexf("-0x1.220c39d4dff5p-61"),
        hi: hexf("0x1.fff555bbb729bp-7"),
    },
    DoubleDouble {
        lo: hexf("-0x1.5ec431444912cp-60"),
        hi: hexf("0x1.ffd55bba97625p-6"),
    },
    DoubleDouble {
        lo: hexf("-0x1.86ef8f794f105p-63"),
        hi: hexf("0x1.7fb818430da2ap-5"),
    },
    DoubleDouble {
        lo: hexf("-0x1.c934d86d23f1dp-60"),
        hi: hexf("0x1.ff55bb72cfdeap-5"),
    },
    DoubleDouble {
        lo: hexf("0x1.ac4ce285df847p-58"),
        hi: hexf("0x1.3f59f0e7c559dp-4"),
    },
    DoubleDouble {
        lo: hexf("-0x1.cfb654c0c3d98p-58"),
        hi: hexf("0x1.7ee182602f10fp-4"),
    },
    DoubleDouble {
        lo: hexf("0x1.f7b8f29a05987p-58"),
        hi: hexf("0x1.be39ebe6f07c3p-4"),
    },
    DoubleDouble {
        lo: hexf("-0x1.cd37686760c17p-59"),
        hi: hexf("0x1.fd5ba9aac2f6ep-4"),
    },
    DoubleDouble {
        lo: hexf("-0x1.b485914dacf8cp-59"),
        hi: hexf("0x1.1e1fafb043727p-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.61a3b0ce9281bp-57"),
        hi: hexf("0x1.3d6eee8c6626cp-3"),
    },
    DoubleDouble {
        lo: hexf("-0x1.054ab2c010f3dp-58"),
        hi: hexf("0x1.5c9811e3ec26ap-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.347b0b4f881cap-58"),
        hi: hexf("0x1.7b97b4bce5b02p-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.cf601e7b4348ep-59"),
        hi: hexf("0x1.9a6a8e96c8626p-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.17b10d2e0e5abp-61"),
        hi: hexf("0x1.b90d7529260a2p-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.c648d1534597ep-57"),
        hi: hexf("0x1.d77d5df205736p-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.8ab6e3cf7afbdp-57"),
        hi: hexf("0x1.f5b75f92c80ddp-3"),
    },
    DoubleDouble {
        lo: hexf("0x1.62e47390cb865p-56"),
        hi: hexf("0x1.09dc597d86362p-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.30ca4748b1bf9p-57"),
        hi: hexf("0x1.18bf5a30bf178p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.077cdd36dfc81p-56"),
        hi: hexf("0x1.278372057ef46p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.963a544b672d8p-57"),
        hi: hexf("0x1.362773707ebccp-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.5d5e43c55b3bap-56"),
        hi: hexf("0x1.44aa436c2af0ap-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.2566480884082p-57"),
        hi: hexf("0x1.530ad9951cd4ap-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.a725715711fp-56"),
        hi: hexf("0x1.614840309cfe2p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.c63aae6f6e918p-56"),
        hi: hexf("0x1.6f61941e4def1p-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.69c885c2b249ap-56"),
        hi: hexf("0x1.7d5604b63b3f7p-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.b6d0ba3748fa8p-56"),
        hi: hexf("0x1.8b24d394a1b25p-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.9e6c988fd0a77p-56"),
        hi: hexf("0x1.98cd5454d6b18p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.24dec1b50b7ffp-56"),
        hi: hexf("0x1.a64eec3cc23fdp-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.ae187b1ca504p-56"),
        hi: hexf("0x1.b3a911da65c6cp-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.cc1ce70934c34p-56"),
        hi: hexf("0x1.c0db4c94ec9fp-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.a2cfa4418f1adp-56"),
        hi: hexf("0x1.cde53432c1351p-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.a2b7f222f65e2p-56"),
        hi: hexf("0x1.dac670561bb4fp-2"),
    },
    DoubleDouble {
        lo: hexf("0x1.0e53dc1bf3435p-56"),
        hi: hexf("0x1.e77eb7f175a34p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.a3992dc382a23p-57"),
        hi: hexf("0x1.f40dd0b541418p-2"),
    },
    DoubleDouble {
        lo: hexf("-0x1.b32c949c9d593p-55"),
        hi: hexf("0x1.0039c73c1a40cp-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.d5b495f6349e6p-56"),
        hi: hexf("0x1.0657e94db30dp-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.974fa13b5404fp-58"),
        hi: hexf("0x1.0c6145b5b43dap-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.2bdaee1c0ee35p-58"),
        hi: hexf("0x1.1255d9bfbd2a9p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.c621cec00c301p-55"),
        hi: hexf("0x1.1835a88be7c13p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.928df287a668fp-58"),
        hi: hexf("0x1.1e00babdefeb4p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.c421c9f38224ep-57"),
        hi: hexf("0x1.23b71e2cc9e6ap-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.09e73b0c6c087p-56"),
        hi: hexf("0x1.2958e59308e31p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.c5d5e9ff0cf8dp-55"),
        hi: hexf("0x1.2ee628406cbcap-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.1021137c71102p-55"),
        hi: hexf("0x1.345f01cce37bbp-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.2304331d8bf46p-55"),
        hi: hexf("0x1.39c391cd4171ap-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.ecf8b492644fp-56"),
        hi: hexf("0x1.3f13fb89e96f4p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.f76d0163f79c8p-56"),
        hi: hexf("0x1.445065b795b56p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.2419a87f2a458p-56"),
        hi: hexf("0x1.4978fa3269ee1p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.4a33dbeb3796cp-55"),
        hi: hexf("0x1.4e8de5bb6ec04p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.1bb74abda520cp-55"),
        hi: hexf("0x1.538f57b89061fp-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.5e5c9d8c5a95p-56"),
        hi: hexf("0x1.587d81f732fbbp-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.0028e4bc5e7cap-57"),
        hi: hexf("0x1.5d58987169b18p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.2b785350ee8c1p-57"),
        hi: hexf("0x1.6220d115d7b8ep-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.6ea6febe8bbbap-56"),
        hi: hexf("0x1.66d663923e087p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.a80386188c50ep-55"),
        hi: hexf("0x1.6b798920b3d99p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.8c34d25aadef6p-56"),
        hi: hexf("0x1.700a7c5784634p-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.7b2a6165884a1p-59"),
        hi: hexf("0x1.748978fba8e0fp-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.406a08980374p-55"),
        hi: hexf("0x1.78f6bbd5d315ep-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.560821e2f3aa9p-55"),
        hi: hexf("0x1.7d528289fa093p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.bf76229d3b917p-56"),
        hi: hexf("0x1.819d0b7158a4dp-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.6b66e7fc8b8c3p-57"),
        hi: hexf("0x1.85d69576cc2c5p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.55b9a5e177a1bp-55"),
        hi: hexf("0x1.89ff5ff57f1f8p-1"),
    },
    DoubleDouble {
        lo: hexf("-0x1.ec182ab042f61p-56"),
        hi: hexf("0x1.8e17aa99cc05ep-1"),
    },
    DoubleDouble {
        lo: hexf("0x1.1a62633145c07p-55"),
        hi: hexf("0x1.921fb54442d18p-1"),
    },
];
