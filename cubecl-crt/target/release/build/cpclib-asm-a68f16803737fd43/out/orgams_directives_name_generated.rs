pub const STAND_ALONE_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b"BANK",
    b"BRK",
    b"BUILDSNA",
    b"BY",
    b"BYTE",
    b"DB",
    b"DEFB",
    b"DEFS",
    b"ELSE",
    b"ENT",
    b"IMPORT",
    b"ORG",
    b"PRINT",
    b"SKIP",
    b"WORD",
];
pub const START_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b"IF",
    b"MACRO",
];
pub const END_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b"END",
    b"ENDM",
    b"]",
];
pub const DOTTED_STAND_ALONE_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b".BANK",
    b".BRK",
    b".BUILDSNA",
    b".BY",
    b".BYTE",
    b".DB",
    b".DEFB",
    b".DEFS",
    b".ELSE",
    b".ENT",
    b".IMPORT",
    b".ORG",
    b".PRINT",
    b".SKIP",
    b".WORD",
];
pub const DOTTED_START_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b".IF",
    b".MACRO",
];
pub const DOTTED_END_DIRECTIVE_ORGAMS: &[&[u8]] = &[
    b".END",
    b".ENDM",
    b".]",
];
