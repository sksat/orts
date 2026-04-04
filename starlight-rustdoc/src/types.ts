/**
 * TypeScript type definitions for rustdoc JSON output (format version 57+).
 *
 * Only the subset of fields actually consumed by this plugin is typed.
 * Unknown fields are silently ignored by JSON.parse.
 */

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

export type Id = number;

export interface Crate {
  root: Id;
  crate_version: string | null;
  includes_private: boolean;
  index: Record<string, Item>;
  paths: Record<string, ItemSummary>;
  external_crates: Record<string, ExternalCrate>;
  format_version: number;
}

export interface ExternalCrate {
  name: string;
  html_root_url: string | null;
}

export interface ItemSummary {
  crate_id: number;
  path: string[];
  kind: ItemKind;
}

export type ItemKind =
  | "module"
  | "struct"
  | "enum"
  | "trait"
  | "function"
  | "type_alias"
  | "constant"
  | "static"
  | "macro"
  | "variant";

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

export interface Item {
  id: Id;
  name: string | null;
  visibility: Visibility;
  docs: string | null;
  attrs: string[];
  deprecation: Deprecation | null;
  inner: ItemInner;
  span: Span | null;
}

export type Visibility =
  | "public"
  | "default"
  | "crate"
  | { restricted: { parent: Id; path: string } };

export interface Deprecation {
  since: string | null;
  note: string | null;
}

export interface Span {
  filename: string;
  begin: [number, number]; // [line, col]
  end: [number, number];
}

// Discriminated union via object keys
export type ItemInner =
  | { module: ModuleItem }
  | { struct: StructItem }
  | { enum: EnumItem }
  | { trait: TraitItem }
  | { function: FunctionItem }
  | { type_alias: TypeAliasItem }
  | { constant: ConstantItem }
  | { static: StaticItem }
  | { impl: ImplItem }
  | { use: UseItem }
  | { struct_field: Type }
  | { variant: VariantItem };

export function getItemKind(inner: ItemInner): string {
  return Object.keys(inner)[0]!;
}

export function getItemData<K extends keyof ItemInnerMap>(
  inner: ItemInner,
  kind: K,
): ItemInnerMap[K] | undefined {
  return (inner as Record<string, unknown>)[kind] as ItemInnerMap[K] | undefined;
}

type ItemInnerMap = {
  module: ModuleItem;
  struct: StructItem;
  enum: EnumItem;
  trait: TraitItem;
  function: FunctionItem;
  type_alias: TypeAliasItem;
  constant: ConstantItem;
  static: StaticItem;
  impl: ImplItem;
  use: UseItem;
  struct_field: Type;
  variant: VariantItem;
};

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

export interface ModuleItem {
  is_crate: boolean;
  items: Id[];
}

// ---------------------------------------------------------------------------
// Struct
// ---------------------------------------------------------------------------

export interface StructItem {
  kind: StructKind;
  generics: Generics;
  impls: Id[];
}

export type StructKind =
  | { plain: { fields: Id[]; has_stripped_fields: boolean } }
  | { tuple: (Id | null)[] }
  | "unit";

// ---------------------------------------------------------------------------
// Enum
// ---------------------------------------------------------------------------

export interface EnumItem {
  generics: Generics;
  variants: Id[];
  impls: Id[];
}

export interface VariantItem {
  kind: VariantKind;
  discriminant: { value: string; expr: string } | null;
}

export type VariantKind =
  | "plain"
  | { tuple: (Id | null)[] }
  | { struct: { fields: Id[]; has_stripped_fields: boolean } };

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

export interface TraitItem {
  is_auto: boolean;
  is_unsafe: boolean;
  is_dyn_compatible: boolean;
  items: Id[];
  generics: Generics;
  bounds: GenericBound[];
  implementations: Id[];
}

// ---------------------------------------------------------------------------
// Function
// ---------------------------------------------------------------------------

export interface FunctionItem {
  sig: FunctionSignature;
  generics: Generics;
  has_body: boolean;
  header: FunctionHeader;
}

export interface FunctionSignature {
  inputs: [string, Type][];
  output: Type | null;
}

export interface FunctionHeader {
  is_const: boolean;
  is_unsafe: boolean;
  is_async: boolean;
  abi: string; // "Rust", "C", etc.
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

export interface ImplItem {
  is_unsafe: boolean;
  generics: Generics;
  provided_trait_methods: string[];
  trait: TypePath | null;
  for: Type;
  items: Id[];
  is_negative: boolean;
  blanket_impl: Type | null;
}

// ---------------------------------------------------------------------------
// Type alias / Constant / Static / Use
// ---------------------------------------------------------------------------

export interface TypeAliasItem {
  generics: Generics;
  type: Type;
}

export interface ConstantItem {
  type: Type;
  const: { expr: string; value: string | null; is_literal: boolean };
}

export interface StaticItem {
  type: Type;
  is_mutable: boolean;
  expr: string;
}

export interface UseItem {
  source: string;
  name: string;
  id: Id | null;
  is_glob: boolean;
}

// ---------------------------------------------------------------------------
// Generics
// ---------------------------------------------------------------------------

export interface Generics {
  params: GenericParam[];
  where_predicates: WherePredicate[];
}

export interface GenericParam {
  name: string;
  kind: GenericParamKind;
}

export type GenericParamKind =
  | { type: { bounds: GenericBound[]; default: Type | null; is_synthetic: boolean } }
  | { lifetime: { outlives: string[] } }
  | { const: { type: Type; default: string | null } };

export type GenericBound = { trait_bound: TraitBound } | { outlives: string } | { use: string[] };

export interface TraitBound {
  trait: TypePath;
  generic_params: GenericParam[];
  modifier: "none" | "maybe" | "const";
}

export type WherePredicate =
  | { bound_predicate: BoundPredicate }
  | { lifetime_predicate: { lifetime: string; outlives: string[] } }
  | { eq_predicate: { lhs: Type; rhs: Type } };

export interface BoundPredicate {
  type: Type;
  bounds: GenericBound[];
  generic_params: GenericParam[];
}

// ---------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------

export type Type =
  | { resolved_path: TypePath }
  | { dyn_trait: DynTrait }
  | { generic: string }
  | { primitive: string }
  | { function_pointer: FunctionPointerType }
  | { tuple: Type[] }
  | { slice: Type }
  | { array: { type: Type; len: string } }
  | { pat: { type: Type; __rest: unknown } }
  | { impl_trait: GenericBound[] }
  | { infer: Record<string, never> }
  | { raw_pointer: { is_mutable: boolean; type: Type } }
  | { borrowed_ref: { lifetime: string | null; is_mutable: boolean; type: Type } }
  | { qualified_path: QualifiedPath };

export interface TypePath {
  path: string;
  id: Id;
  args: GenericArgs | null;
}

export interface QualifiedPath {
  name: string;
  args: GenericArgs | null;
  self_type: Type;
  trait: TypePath | null;
}

export type GenericArgs =
  | { angle_bracketed: AngleBracketedArgs }
  | { parenthesized: ParenthesizedArgs };

export interface AngleBracketedArgs {
  args: GenericArg[];
  constraints: TypeBindingConstraint[];
}

export type GenericArg =
  | { lifetime: string }
  | { type: Type }
  | { const: { expr: string; value: string | null; is_literal: boolean } }
  | { infer: Record<string, never> };

export interface TypeBindingConstraint {
  name: string;
  args: GenericArgs | null;
  binding: { equality: GenericArg } | { constraint: GenericBound[] };
}

export interface ParenthesizedArgs {
  inputs: Type[];
  output: Type | null;
}

export interface DynTrait {
  traits: PolyTrait[];
  lifetime: string | null;
}

export interface PolyTrait {
  trait: TypePath;
  generic_params: GenericParam[];
}

export interface FunctionPointerType {
  sig: FunctionSignature;
  generic_params: GenericParam[];
  header: FunctionHeader;
}
