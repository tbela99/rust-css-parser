// exports shared by node.js and web.js
export { RenderResult, TransformResult } from "./printer/result.js";
export { minify } from "./ast/minify.js";
export { parseDeclarations, parseString } from "./parser/parse.js";
export { renderValue } from "./printer/render.js";
export { convertColor } from "./syntax/color/color.js";
export { isOkLabClose, okLabDistance } from "./syntax/color/utils/distance.js";
export { Arena, AstArena, TokenField, arenaColumns } from "./ast/arena.js";
export { ARENA, ARENAID, AstArenaNode } from "./ast/arena-node.js";
export { ColorType, EnumAstNodeStatus, EnumToken, MergeRulesEnum, ModuleCaseTransformEnum, ModuleScopeEnumOptions, WalkerEvent, WalkerOptionEnum } from "./ast/types.js";
export { SourceMap } from "./printer/sourcemap.js";
export { FeatureWalkMode } from "./ast/features/type.js";
export { parseSyntax } from "./validation/parser/parse.js";
export { ResponseType } from "./enums-extra.js";
export { dirname, resolve } from "./fs/resolve.js";
export { version } from "./version.js";
