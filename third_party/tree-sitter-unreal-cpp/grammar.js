/**
 * @file C++ grammar for tree-sitter
 * @author Max Brunsfeld <maxbrunsfeld@gmail.com>
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @author John Drouhard <john@drouhard.dev>
 * @author Pablo Hugen <pabloashugen@protonmail.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const C = require('tree-sitter-c/grammar');

const PREC = Object.assign(C.PREC, {
  LAMBDA: 18,
  NEW: C.PREC.CALL + 1,
  STRUCTURED_BINDING: -1,
  THREE_WAY: C.PREC.RELATIONAL + 1,
});

const FOLD_OPERATORS = [
  '+', '-', '*', '/', '%',
  '^', '&', '|',
  '=', '<', '>',
  '<<', '>>',
  '+=', '-=', '*=', '/=', '%=', '^=', '&=', '|=',
  '>>=', '<<=',
  '==', '!=', '<=', '>=',
  '&&', '||',
  ',',
  '.*', '->*',
  'or', 'and', 'bitor', 'xor', 'bitand', 'not_eq',
];

const ASSIGNMENT_OPERATORS = [
  '=',
  '*=',
  '/=',
  '%=',
  '+=',
  '-=',
  '<<=',
  '>>=',
  '&=',
  '^=',
  '|=',
  'and_eq',
  'or_eq',
  'xor_eq',
];

const commaSep = C.commaSep;
const commaSep1 = C.commaSep1;
const preprocIf = C.preprocIf;

module.exports = grammar(C, {
  name: 'cpp',

  externals: $ => [
    $.raw_string_delimiter,
    $.raw_string_content,
  ],

      conflicts: $ => [
        // C
        [$.type_specifier, $._declarator],
        [$.type_specifier, $.expression],
        [$.sized_type_specifier],
        [$.attributed_statement],
        [$._declaration_modifiers, $.attributed_statement],
        [$._declaration_modifiers, $.using_declaration],
        [$._declaration_modifiers, $.attributed_statement, $.using_declaration],
        [$._top_level_item, $._top_level_statement],
        [$._block_item, $.statement],
        [$.type_qualifier, $.extension_expression],
  
        // C++
        [$.template_function, $.template_type],
        [$.template_function, $.template_type, $.expression],
        [$.template_function, $.template_type, $.qualified_identifier],
        [$.template_function, $.template_type, $.qualified_identifier, $.qualified_type_identifier],
        [$.template_type, $.qualified_type_identifier],
        [$.qualified_type_identifier, $.qualified_identifier],
        [$.comma_expression, $.initializer_list],
        [$.expression, $._declarator],
        [$.expression, $.structured_binding_declarator],
        [$.expression, $._declarator, $.type_specifier],
        [$.expression, $.identifier_parameter_pack_expansion],
        [$.expression, $._lambda_capture_identifier],
        [$.expression, $._lambda_capture],
        [$.expression, $.structured_binding_declarator, $._lambda_capture_identifier],
        [$.structured_binding_declarator, $._lambda_capture_identifier],
        [$.parameter_list, $.argument_list],
        [$.type_specifier, $.call_expression],
        [$._declaration_specifiers, $._constructor_specifiers],
        [$._binary_fold_operator, $._fold_operator],
        [$._function_declarator_seq],
        [$.type_specifier, $.sized_type_specifier],
        [$.initializer_pair, $.comma_expression],
        [$.expression_statement, $._for_statement_body],
        [$.init_statement, $._for_statement_body],
        [$.field_expression, $.template_method, $.template_type],
        [$.field_expression, $.template_method],
        [$.qualified_field_identifier, $.template_method, $.template_type],
        [$.type_specifier, $.template_type, $.template_function, $.expression],
        [$.splice_type_specifier, $.splice_expression],
  
        //unreal
        // [$.storage_class_specifier, $.expression],
        [$.storage_class_specifier, $.expression, $._constructor_specifiers],
        [$.storage_class_specifier, $._constructor_specifiers],
          // ▼▼▼ 追加: UE_DEPRECATED の競合解決 ▼▼▼
              [$.declaration, $._declaration_modifiers],
              [$.unreal_class_declaration],
              [$.unreal_struct_declaration],
                  [$.unreal_enum_declaration],
                  // [$.field_declaration, $._declaration_modifiers], 
               
                                  // ▲▲▲ 追加完了 ▲▲▲  //
      
      ],
    inline: ($, original) => original.concat([
    $._namespace_identifier,
  ]),

  precedences: $ => [
    [$.argument_list, $.type_qualifier],
    [$._expression_not_binary, $._class_name],
  ],

  rules: {

    _top_level_item: ($, original) => choice(
      prec(10000, $.unreal_declaration_macro), // 最優先に引き上げ
      //start unreal engien
      prec(100, $.unreal_class_declaration),
      prec(100, $.unreal_struct_declaration),
      prec(100, $.unreal_enum_declaration), 
      //end unreal engine
      ...original.members.filter((member) => member.content?.name != '_old_style_function_definition'),
      $.namespace_definition,
      $.concept_definition,
      $.namespace_alias_definition,
      $.using_declaration,
      $.alias_declaration,
      $.static_assert_declaration,
      $.consteval_block_declaration,
      $.template_declaration,
      $.template_instantiation,
      $.module_declaration,
      $.export_declaration,
      $.import_declaration,
      $.global_module_fragment_declaration,
      $.private_module_fragment_declaration,
      alias($.constructor_or_destructor_definition, $.function_definition),
      alias($.operator_cast_definition, $.function_definition),
      alias($.operator_cast_declaration, $.declaration),


      //unreal
      $.unreal_pragma_macro,
      //unreal
    ),

    preproc_directive: ($, original) => choice(
      // #pragma 専用のルール
      seq(
        '#',
        // 'pragma' という文字列を 'identifier' として扱う
        alias($._pragma_directive_identifier, $.identifier),
        // 新しく作る '_pragma_argument' トークンを 'preproc_arg' としてエイリアスする
        optional(alias($._pragma_argument, $.preproc_arg)),
        '\n'
      ),
      // #pragma 以外のディレクティブは、オリジナルのC文法に任せる
      original 
    ),
    _pragma_directive_identifier: _ => 'pragma',


    enumerator: ($, original) => seq(
      original,
      optional($.umeta_macro)
    ),


    _block_item: ($, original) => choice(
      ...original.members.filter((member) =>
        member.content?.name != '_old_style_function_definition' &&
        !member.name.startsWith('preproc_if'),
      ),
      $.namespace_definition,
      $.concept_definition,
      $.namespace_alias_definition,
      $.using_declaration,
      $.alias_declaration,
      $.static_assert_declaration,
      $.consteval_block_declaration,
      $.template_declaration,
      $.template_instantiation,
      $.export_declaration,
      $.import_declaration,
      alias($.preproc_if_in_block, $.preproc_if),
      alias($.preproc_ifdef_in_block, $.preproc_ifdef),
      alias($.constructor_or_destructor_definition, $.function_definition),
      alias($.operator_cast_definition, $.function_definition),
      alias($.operator_cast_declaration, $.declaration),

      // ▼▼▼ 追加: 関数内での PRAGMA マクロを許可 ▼▼▼
      $.unreal_pragma_macro,
      // ▲▲▲ 追加完了 ▲▲▲
    ),

    ...preprocIf('', $ => $._top_level_item),
    ...preprocIf('_in_block', $ => $._block_item),

    // Types

    placeholder_type_specifier: $ => prec(1, seq(
      field('constraint', optional(choice(
        alias($.qualified_type_identifier, $.qualified_identifier),
        $.template_type,
        $._type_identifier,
      ))),
      choice($.auto, alias($.decltype_auto, $.decltype)),
    )),

    auto: _ => 'auto',
    decltype_auto: $ => seq(
      'decltype',
      '(',
      $.auto,
      ')',
    ),
    decltype: $ => seq(
      'decltype',
      '(',
      $.expression,
      ')',
    ),

    type_specifier: $ => choice(
      $.struct_specifier,
      $.union_specifier,
      $.enum_specifier,
      $.class_specifier,
      $.sized_type_specifier,
      $.primitive_type,
      $.template_type,
      $.dependent_type,
      $.splice_type_specifier,
      $.placeholder_type_specifier,
      $.decltype,
      prec.right(choice(
        alias($.qualified_type_identifier, $.qualified_identifier),
        $._type_identifier,
      )),
    ),

    type_qualifier: (_, original) => choice(
      original,
      'mutable',
      'constinit',
      'consteval',
    ),

    type_descriptor: (_, original) => prec.right(original),

    attribute: ($, original) => seq(
      optional(seq('using', field('namespace', $.identifier), ':')),
      ...original.members,
    ),

    annotation: $ => seq('=', $.expression),

    attribute_declaration: ($, original) => choice(
      original,
      seq('[[', commaSep1($.annotation), ']]'),
    ),

    // When used in a trailing return type, these specifiers can now occur immediately before
    // a compound statement. This introduces a shift/reduce conflict that needs to be resolved
    // with an associativity.
    _class_declaration: $ => seq(
      repeat(choice($.attribute_specifier, $.alignas_qualifier)),
      optional($.ms_declspec_modifier),
      repeat($.attribute_declaration),
      $._class_declaration_item,
    ),
    _class_declaration_item: $ => prec.right(seq(
      choice(
        seq(
          optional($.unreal_api_specifier), // ここに追加！
          field('name', $._class_name)
        ),
        seq(
          optional($.unreal_api_specifier), // ここにも追加
          optional(field('name', $._class_name)),
          optional($.virtual_specifier),
          optional($.base_class_clause),
          field('body', $.field_declaration_list),
        ),
      ),
      optional($.attribute_specifier),
    )),

    class_specifier: $ => seq(
      'class',
      $._class_declaration,
    ),

    union_specifier: $ => seq(
      'union',
      $._class_declaration,
    ),

    struct_specifier: $ => seq(
      'struct',
      $._class_declaration,
    ),

    _class_name: $ => prec.right(choice(
      $._type_identifier,
      $.template_type,
      $.splice_type_specifier,
      alias($.qualified_type_identifier, $.qualified_identifier),
    )),

    function_definition: ($, original) => ({
      ...original,
      members: original.members.map(
        (e) => e.name !== 'body' ?
          e :
          field('body', choice(e.content, $.try_statement))),
    }),

    declaration: $ => seq(
      // ▼▼▼ 追加: DEPRECATEDマクロを許可 ▼▼▼
      optional($.unreal_deprecated_macro),
      // ▲▲▲ 追加完了 ▲▲▲
      $._declaration_specifiers,
      commaSep1(field('declarator', choice(
        seq(
          // C uses _declaration_declarator here for some nice macro parsing in function declarators,
          // but this causes a world of pain for C++ so we'll just stick to the normal _declarator here.
          optional($.ms_call_modifier),
          $._declarator,
          optional($.gnu_asm_expression),
        ),
        $.init_declarator,
      ))),
      ';',
    ),

    virtual_specifier: _ => choice(
      'final', // the only legal value here for classes
      'override', // legal for functions in addition to final, plus permutations.
    ),

    _declaration_modifiers: ($, original) => choice(
      original,
      'virtual',
      // ▼▼▼ 追加: あらゆる宣言で DEPRECATED マクロを許可する ▼▼▼
      $.unreal_deprecated_macro,
      // ▲▲▲ 追加完了 ▲▲▲
    ),

    explicit_function_specifier: $ => choice(
      'explicit',
      prec(PREC.CALL, seq(
        'explicit',
        '(',
        $.expression,
        ')',
      )),
    ),

    base_class_clause: $ => seq(
      ':',
      commaSep1(seq(
        repeat($.attribute_declaration),
        optional(choice(
          $.access_specifier,
          seq($.access_specifier, optional('virtual')),
          seq('virtual', optional($.access_specifier)),
        )),
        $._class_name,
        optional('...'),
      )),
    ),

    enum_specifier: $ => prec.right(seq(
      'enum',
      optional(choice('class', 'struct')),
      optional($.unreal_api_specifier), // ここに追加！
      choice(
        seq(
          field('name', $._class_name),
          optional($._enum_base_clause),
          optional(field('body', $.enumerator_list)),
        ),
        field('body', $.enumerator_list),
      ),
      optional($.attribute_specifier),
    )),


    _enum_base_clause: $ => prec.left(seq(
      ':',
      field('base', choice(
        alias($.qualified_type_identifier, $.qualified_identifier),
        $._type_identifier,
        $.primitive_type,
        $.sized_type_specifier,
      )),
    )),

    // The `auto` storage class is removed in C++0x in order to allow for the `auto` type.
    storage_class_specifier: ($, original) => choice(
      ...original.members.filter((member) => member.value !== 'auto'),
      'thread_local',
      //unreal
      $.unreal_api_specifier, // <-- これを追加
      $.unreal_force_inline, // <<< これを追加
      //unreal
    ),

    dependent_type: $ => prec.dynamic(-1, prec.right(seq(
      'typename',
      $.type_specifier,
    ))),

    // Declarations

    module_name: $ => seq(
      $.identifier,
      repeat(seq('.', $.identifier),
      ),
    ),

    module_partition: $ => seq(
      ':',
      $.module_name,
    ),

    module_declaration: $ => seq(
      optional('export'),
      'module',
      field('name', $.module_name),
      field('partition', optional($.module_partition)),
      optional($.attribute_declaration),
      ';',
    ),

    export_declaration: $ => prec(1, seq(
      'export',
      choice($._block_item, $.declaration_list),
    )),

    import_declaration: $ => seq(
      'import',
      choice(
        field('name', $.module_name),
        field('partition', $.module_partition),
        field('header', choice(
          $.string_literal,
          $.system_lib_string,
        )),
      ),
      optional($.attribute_declaration),
      ';',
    ),

    global_module_fragment_declaration: _ => seq('module', ';'),
    private_module_fragment_declaration: _ => seq('module', ':', 'private', ';'),

    template_declaration: $ => seq(
      'template',
      field('parameters', $.template_parameter_list),
      optional($.requires_clause),
      choice(
        $._empty_declaration,
        $.alias_declaration,
        $.declaration,
        $.template_declaration,
        $.function_definition,
        $.concept_definition,
        $.friend_declaration,
        alias($.constructor_or_destructor_declaration, $.declaration),
        alias($.constructor_or_destructor_definition, $.function_definition),
        alias($.operator_cast_declaration, $.declaration),
        alias($.operator_cast_definition, $.function_definition),
      ),
    ),

    template_instantiation: $ => prec(1, seq(
      optional('extern'),
      'template',
      optional($._declaration_specifiers),
      field('declarator', $._declarator),
      ';',
    )),

    template_parameter_list: $ => seq(
      '<',
      commaSep(choice(
        $.parameter_declaration,
        $.optional_parameter_declaration,
        $.type_parameter_declaration,
        $.variadic_parameter_declaration,
        $.variadic_type_parameter_declaration,
        $.optional_type_parameter_declaration,
        $.template_template_parameter_declaration,
      )),
      alias(token(prec(1, '>')), '>'),
    ),

    type_parameter_declaration: $ => prec(1, seq(
      choice('typename', 'class'),
      optional($._type_identifier),
    )),

    variadic_type_parameter_declaration: $ => prec(1, seq(
      choice('typename', 'class'),
      '...',
      optional($._type_identifier),
    )),

    optional_type_parameter_declaration: $ => seq(
      choice('typename', 'class'),
      optional(field('name', $._type_identifier)),
      '=',
      field('default_type', $.type_specifier),
    ),

    template_template_parameter_declaration: $ => seq(
      'template',
      field('parameters', $.template_parameter_list),
      choice(
        $.type_parameter_declaration,
        $.variadic_type_parameter_declaration,
        $.optional_type_parameter_declaration,
      ),
    ),

    parameter_list: $ => seq(
      '(',
      commaSep(choice(
        $.parameter_declaration,
        $.explicit_object_parameter_declaration,
        $.optional_parameter_declaration,
        $.variadic_parameter_declaration,
        '...',
      )),
      ')',
    ),

    explicit_object_parameter_declaration: $ => seq(
      $.this,
      $.parameter_declaration,
    ),

    optional_parameter_declaration: $ => seq(
      $._declaration_specifiers,
      field('declarator', optional(choice($._declarator, $.abstract_reference_declarator))),
      '=',
      field('default_value', $.expression),
    ),

    variadic_parameter_declaration: $ => seq(
      $._declaration_specifiers,
      field('declarator', choice(
        $.variadic_declarator,
        alias($.variadic_reference_declarator, $.reference_declarator),
      )),
    ),

    variadic_declarator: $ => seq(
      '...',
      optional($.identifier),
    ),

    variadic_reference_declarator: $ => seq(
      choice('&&', '&'),
      $.variadic_declarator,
    ),

    init_declarator: ($, original) => choice(
      original,
      seq(
        field('declarator', $._declarator),
        field('value', choice(
          $.argument_list,
          $.initializer_list,
        )),
      ),
    ),

    operator_cast: $ => prec.right(1, seq(
      'operator',
      $._declaration_specifiers,
      field('declarator', $._abstract_declarator),
    )),

    // Avoid ambiguity between compound statement and initializer list in a construct like:
    //   A b {};
    compound_statement: (_, original) => prec(-1, original),

    field_initializer_list: $ => seq(
      ':',
      commaSep1($.field_initializer),
    ),

    field_initializer: $ => prec(1, seq(
      choice(
        $._field_identifier,
        $.template_method,
        alias($.qualified_field_identifier, $.qualified_identifier),
      ),
      choice($.initializer_list, $.argument_list),
      optional('...'),
    )),

    _field_declaration_list_item: ($, original) => choice(
      $.unreal_body_macro,
      $.unreal_declare_class_macro,
      $.unreal_define_default_object_initializer_macro,
      prec(100, $.unreal_declaration_macro), // ここに移動し、優先度を上げる
// ▼▼▼ 追加: UENUM, USTRUCT, UCLASS, UFUNCTION を許可 ▼▼▼
      $.unreal_enum_declaration,
      $.unreal_struct_declaration,
      $.unreal_class_declaration,
      $.unreal_function_declaration, // <--- これを追加！
      $.unreal_pragma_macro,
      // ▲▲▲ 追加完了 ▲▲▲
      original,
      $.template_declaration,
      alias($.inline_method_definition, $.function_definition),
      alias($.constructor_or_destructor_definition, $.function_definition),
      alias($.constructor_or_destructor_declaration, $.declaration),
      alias($.operator_cast_definition, $.function_definition),
      alias($.operator_cast_declaration, $.declaration),
      $.friend_declaration,
      seq($.access_specifier, ':'),
      $.alias_declaration,
      $.using_declaration,
      $.type_definition,
      $.static_assert_declaration,
      $.consteval_block_declaration,
      ';',

      //unreael
      $.unreal_declaration_macro,
      //unreal
    ),

    // ---------------------------------------------------------
    // 2. field_declaration から ufunction_macro を削除
    // ---------------------------------------------------------
    field_declaration: $ => prec(10, seq(
      //unreal
      optional($.unreal_deprecated_macro),
      
      // ▼▼▼ 修正: ここから $.ufunction_macro を削除 (UPROPERTY専用にする) ▼▼▼
      optional($.uproperty_macro),
      // ▲▲▲ 修正完了 ▲▲▲

      //unreal
      $._declaration_specifiers,
      commaSep(seq(
        field('declarator', $._field_declarator),
        optional(choice(
          $.bitfield_clause,
          field('default_value', $.initializer_list),
          seq('=', field('default_value', choice($.expression, $.initializer_list))),
        )),
      )),
      optional($.attribute_specifier),
      ';'
    )),
    // ---------------------------------------------------------
    // 3. 新規追加: UFUNCTION 専用の宣言ルール (プロトタイプ)
    // ---------------------------------------------------------
    unreal_function_declaration: $ => seq(
      optional($.unreal_deprecated_macro),
      $.ufunction_macro,
      $._declaration_specifiers,
      field('declarator', $._field_declarator),
      optional($.attribute_specifier),
      optional(choice(
         $.default_method_clause,
         $.delete_method_clause,
         $.pure_virtual_clause
      )),
      ';'
    ),
    inline_method_definition: $ => prec(1, seq(
      //unreal
      optional($.unreal_deprecated_macro),
      optional($.ufunction_macro),
      //unreal
      $._declaration_specifiers,
      field('declarator', $._field_declarator),
      choice(
        field('body', choice($.compound_statement, $.try_statement)),
        $.default_method_clause,
        $.delete_method_clause,
        $.pure_virtual_clause,
      ),
    )),

    _constructor_specifiers: $ => choice(
      $._declaration_modifiers,
      $.explicit_function_specifier,
      // ▼▼▼ 追加: コンストラクタにも API マクロと FORCEINLINE を許可する ▼▼▼
      $.unreal_api_specifier,
      $.unreal_force_inline,
      // ▲▲▲ 追加完了 ▲▲▲
    ),

    operator_cast_definition: $ => seq(
      repeat($._constructor_specifiers),
      field('declarator', choice(
        $.operator_cast,
        alias($.qualified_operator_cast_identifier, $.qualified_identifier),
      )),
      field('body', choice($.compound_statement, $.try_statement)),
    ),

    operator_cast_declaration: $ => prec(1, seq(
      repeat($._constructor_specifiers),
      field('declarator', choice(
        $.operator_cast,
        alias($.qualified_operator_cast_identifier, $.qualified_identifier),
      )),
      optional(seq('=', field('default_value', $.expression))),
      ';',
    )),

    constructor_try_statement: $ => seq(
      'try',
      optional($.field_initializer_list),
      field('body', $.compound_statement),
      repeat1($.catch_clause),
    ),

    constructor_or_destructor_definition: $ => seq(
      repeat($._constructor_specifiers),
      field('declarator', $.function_declarator),
      choice(
        seq(
          optional($.field_initializer_list),
          field('body', $.compound_statement),
        ),
        alias($.constructor_try_statement, $.try_statement),
        $.default_method_clause,
        $.delete_method_clause,
        $.pure_virtual_clause,
      ),
    ),

    constructor_or_destructor_declaration: $ => seq(
      repeat($._constructor_specifiers),
      field('declarator', $.function_declarator),
      ';',
    ),

    default_method_clause: _ => seq('=', 'default', ';'),
    delete_method_clause: _ => seq('=', 'delete', ';'),
    pure_virtual_clause: _ => seq('=', /0/, ';'),

    friend_declaration: $ => seq(
      optional('constexpr'),
      'friend',
      choice(
        $.declaration,
        $.function_definition,
        seq(
          optional(choice(
            'class',
            'struct',
            'union',
          )),
          $._class_name, ';',
        ),
      ),
    ),

    access_specifier: _ => choice(
      'public',
      'private',
      'protected',
    ),

    _declarator: ($, original) => choice(
      original,
      $.reference_declarator,
      $.qualified_identifier,
      $.template_function,
      $.operator_name,
      $.destructor_name,
      $.structured_binding_declarator,
    ),

    _field_declarator: ($, original) => choice(
      original,
      alias($.reference_field_declarator, $.reference_declarator),
      $.template_method,
      $.operator_name,
    ),

    _type_declarator: ($, original) => choice(
      original,
      alias($.reference_type_declarator, $.reference_declarator),
    ),

    _abstract_declarator: ($, original) => choice(
      original,
      $.abstract_reference_declarator,
    ),

    reference_declarator: $ => prec.dynamic(1, prec.right(seq(choice('&', '&&'), $._declarator))),
    reference_field_declarator: $ => prec.dynamic(1, prec.right(seq(choice('&', '&&'), $._field_declarator))),
    reference_type_declarator: $ => prec.dynamic(1, prec.right(seq(choice('&', '&&'), $._type_declarator))),
    abstract_reference_declarator: $ => prec.right(seq(choice('&', '&&'), optional($._abstract_declarator))),

    structured_binding_declarator: $ => prec.dynamic(PREC.STRUCTURED_BINDING, seq(
      '[', commaSep1($.identifier), ']',
    )),

    ref_qualifier: _ => choice('&', '&&'),

    _function_declarator_seq: $ => seq(
      field('parameters', $.parameter_list),
      optional($._function_attributes_start),
      optional($.ref_qualifier),
      optional($._function_exception_specification),
      optional($._function_attributes_end),
      optional($.trailing_return_type),
      optional($._function_postfix),
    ),

    _function_attributes_start: $ => prec(1, choice(
      seq(repeat1($.attribute_specifier), repeat($.type_qualifier)),
      seq(repeat($.attribute_specifier), repeat1($.type_qualifier)),
    )),

    _function_exception_specification: $ => choice(
      $.noexcept,
      $.throw_specifier,
    ),

    _function_attributes_end: $ => prec.right(seq(
      optional($.gnu_asm_expression),
      choice(
        seq(repeat1($.attribute_specifier), repeat($.attribute_declaration)),
        seq(repeat($.attribute_specifier), repeat1($.attribute_declaration)),
      ),
    )),

    _function_postfix: $ => prec.right(choice(
      repeat1($.virtual_specifier),
      $.requires_clause,
    )),

    function_declarator: $ => prec.dynamic(1, seq(
      field('declarator', $._declarator),
      $._function_declarator_seq,
    )),

    function_field_declarator: $ => prec.dynamic(1, seq(
      field('declarator', $._field_declarator),
      $._function_declarator_seq,
    )),

    abstract_function_declarator: $ => seq(
      field('declarator', optional($._abstract_declarator)),
      $._function_declarator_seq,
    ),

    trailing_return_type: $ => seq('->', $.type_descriptor),

    noexcept: $ => prec.right(seq(
      'noexcept',
      optional(
        seq(
          '(',
          optional($.expression),
          ')',
        ),
      ),
    )),

    throw_specifier: $ => seq(
      'throw',
      seq(
        '(',
        commaSep($.type_descriptor),
        ')',
      ),
    ),

    template_type: $ => seq(
      field('name', $._type_identifier),
      field('arguments', $.template_argument_list),
    ),

    template_method: $ => seq(
      field('name', choice($._field_identifier, $.operator_name)),
      field('arguments', $.template_argument_list),
    ),

    template_function: $ => seq(
      field('name', $.identifier),
      field('arguments', $.template_argument_list),
    ),

    template_argument_list: $ => seq(
      '<',
      commaSep(choice(
        prec.dynamic(3, $.type_descriptor),
        prec.dynamic(2, alias($.type_parameter_pack_expansion, $.parameter_pack_expansion)),
        prec.dynamic(1, $.expression),
      )),
      alias(token(prec(1, '>')), '>'),
    ),

    namespace_definition: $ => seq(
      optional('inline'),
      'namespace',
      optional($.attribute_declaration),
      field('name', optional(
        choice(
          $._namespace_identifier,
          $.nested_namespace_specifier,
        ))),
      field('body', $.declaration_list),
    ),

    namespace_alias_definition: $ => seq(
      'namespace',
      field('name', $._namespace_identifier),
      '=',
      choice(
        $._namespace_identifier,
        $.nested_namespace_specifier,
        $.splice_specifier,
      ),
      ';',
    ),

    _namespace_specifier: $ => seq(
      optional('inline'),
      $._namespace_identifier,
    ),

    nested_namespace_specifier: $ => prec(1, seq(
      optional($._namespace_specifier),
      '::',
      choice(
        $.nested_namespace_specifier,
        $._namespace_specifier,
      ),
    )),

    using_declaration: $ => seq(
      repeat($.attribute_declaration),
      'using',
      optional(choice('namespace', 'enum')),
      choice(
        $.identifier,
        $.qualified_identifier,
        $.splice_type_specifier,
      ),
      ';',
    ),

    alias_declaration: $ => seq(
      'using',
      field('name', $._type_identifier),
      repeat($.attribute_declaration),
      '=',
      field('type', $.type_descriptor),
      ';',
    ),

    static_assert_declaration: $ => seq(
      'static_assert',
      '(',
      field('condition', $.expression),
      optional(seq(
        ',',
        field('message', $._string),
      )),
      ')',
      ';',
    ),

    consteval_block_declaration: $ => seq(
      'consteval',
      field('body', $.compound_statement),
    ),

    concept_definition: $ => seq(
      'concept',
      field('name', $.identifier),
      '=',
      $.expression,
      ';',
    ),

    // Statements

    _top_level_statement: ($, original) => choice(
      original,
      $.co_return_statement,
      $.co_yield_statement,
      $.for_range_loop,
      $.expansion_statement,
      $.try_statement,
      $.throw_statement,
    ),

    _non_case_statement: ($, original) => choice(
      original,
      $.co_return_statement,
      $.co_yield_statement,
      $.for_range_loop,
      $.expansion_statement,
      $.try_statement,
      $.throw_statement,
    ),

    switch_statement: $ => seq(
      'switch',
      field('condition', $.condition_clause),
      field('body', $.compound_statement),
    ),

    while_statement: $ => seq(
      'while',
      field('condition', $.condition_clause),
      field('body', $.statement),
    ),

    if_statement: $ => prec.right(seq(
      'if',
      optional('constexpr'),
      field('condition', $.condition_clause),
      field('consequence', $.statement),
      optional(field('alternative', $.else_clause)),
    )),

    // Using prec(1) instead of prec.dynamic(1) causes issues with the
    // range loop's declaration specifiers if `int` is passed in, it'll
    // always prefer the standard for loop and give us a parse error.
    _for_statement_body: ($, original) => prec.dynamic(1, original),
    for_range_loop: $ => seq(
      'for',
      '(',
      $._for_range_loop_body,
      ')',
      field('body', $.statement),
    ),
    _for_range_loop_body: $ => seq(
      field('initializer', optional($.init_statement)),
      $._declaration_specifiers,
      field('declarator', $._declarator),
      ':',
      field('right', choice(
        $.expression,
        $.initializer_list,
      )),
    ),

    init_statement: $ => choice(
      $.alias_declaration,
      $.type_definition,
      $.declaration,
      $.expression_statement,
    ),

    condition_clause: $ => seq(
      '(',
      field('initializer', optional($.init_statement)),
      field('value', choice(
        $.expression,
        $.comma_expression,
        alias($.condition_declaration, $.declaration),
      )),
      ')',
    ),

    condition_declaration: $ => seq(
      $._declaration_specifiers,
      field('declarator', $._declarator),
      choice(
        seq(
          '=',
          field('value', $.expression),
        ),
        field('value', $.initializer_list),
      ),
    ),

    return_statement: ($, original) => seq(
      choice(
        original,
        seq('return', $.initializer_list, ';'),
      ),
    ),

    co_return_statement: $ => seq(
      'co_return',
      optional($.expression),
      ';',
    ),

    co_yield_statement: $ => seq(
      'co_yield',
      $.expression,
      ';',
    ),

    throw_statement: $ => seq(
      'throw',
      optional($.expression),
      ';',
    ),

    try_statement: $ => seq(
      'try',
      field('body', $.compound_statement),
      repeat1($.catch_clause),
    ),

    catch_clause: $ => seq(
      'catch',
      field('parameters', $.parameter_list),
      field('body', $.compound_statement),
    ),

    // Expressions

    _expression_not_binary: ($, original) => choice(
      original,
      $.co_await_expression,
      $.requires_expression,
      $.requires_clause,
      $.template_function,
      $.qualified_identifier,
      $.new_expression,
      $.delete_expression,
      $.lambda_expression,
      $.parameter_pack_expansion,
      $.this,
      $.user_defined_literal,
      $.fold_expression,
      $.reflect_expression,
      $.splice_expression,
      // unreal
      $.unreal_api_specifier,
      //unreal
    ),

    _string: $ => choice(
      $.string_literal,
      $.raw_string_literal,
      $.concatenated_string,
    ),

    raw_string_literal: $ => seq(
      choice('R"', 'LR"', 'uR"', 'UR"', 'u8R"'),
      choice(
        seq(
          field('delimiter', $.raw_string_delimiter),
          '(',
          $.raw_string_content,
          ')',
          $.raw_string_delimiter,
        ),
        seq('(', $.raw_string_content, ')'),
      ),
      '"',
    ),

    subscript_expression: $ => prec(PREC.SUBSCRIPT, seq(
      field('argument', $.expression),
      field('indices', $.subscript_argument_list),
    )),

    subscript_argument_list: $ => seq(
      '[',
      commaSep(choice($.expression, $.initializer_list)),
      ']',
    ),

    call_expression: ($, original) => prec.dynamic(1, choice(original, seq(
      field('function', choice($.primitive_type, seq(
        optional('typename'),
        $.splice_type_specifier,
      ))),
      field('arguments', $.argument_list),
    ))),

    co_await_expression: $ => prec.left(PREC.UNARY, seq(
      field('operator', 'co_await'),
      field('argument', $.expression),
    )),

    new_expression: $ => prec.right(PREC.NEW, seq(
      optional('::'),
      'new',
      field('placement', optional($.argument_list)),
      field('type', $.type_specifier),
      field('declarator', optional($.new_declarator)),
      field('arguments', optional(choice(
        $.argument_list,
        $.initializer_list,
      ))),
    )),

    new_declarator: $ => prec.right(seq(
      '[',
      field('length', $.expression),
      ']',
      optional($.new_declarator),
    )),

    delete_expression: $ => seq(
      optional('::'),
      'delete',
      optional(seq('[', ']')),
      $.expression,
    ),

    field_expression: $ => seq(
      prec(PREC.FIELD, seq(
        field('argument', $.expression),
        field('operator', choice('.', '.*', '->')),
      )),
      field('field', choice(
        prec.dynamic(1, $._field_identifier),
        alias($.qualified_field_identifier, $.qualified_identifier),
        $.destructor_name,
        $.template_method,
        alias($.dependent_field_identifier, $.dependent_name),
        $.operator_name,
        $.splice_expression,
      )),
    ),

    type_requirement: $ => seq('typename', $._class_name),

    compound_requirement: $ => seq(
      '{', $.expression, '}',
      optional('noexcept'),
      optional($.trailing_return_type),
      ';',
    ),

    _requirement: $ => choice(
      alias($.expression_statement, $.simple_requirement),
      $.type_requirement,
      $.compound_requirement,
    ),

    requirement_seq: $ => seq('{', repeat($._requirement), '}'),

    constraint_conjunction: $ => prec.left(PREC.LOGICAL_AND, seq(
      field('left', $._requirement_clause_constraint),
      field('operator', choice('&&', 'and')),
      field('right', $._requirement_clause_constraint)),
    ),

    constraint_disjunction: $ => prec.left(PREC.LOGICAL_OR, seq(
      field('left', $._requirement_clause_constraint),
      field('operator', choice('||', 'or')),
      field('right', $._requirement_clause_constraint)),
    ),

    _requirement_clause_constraint: $ => choice(
      // Primary expressions"
      $.true,
      $.false,
      $._class_name,
      $.fold_expression,
      $.lambda_expression,
      $.requires_expression,

      // Parenthesized expressions
      seq('(', $.expression, ')'),

      // conjunction or disjunction of the above
      $.constraint_conjunction,
      $.constraint_disjunction,
    ),

    requires_clause: $ => seq(
      'requires',
      field('constraint', $._requirement_clause_constraint),
    ),

    requires_parameter_list: $ => seq(
      '(',
      commaSep(choice(
        $.parameter_declaration,
        $.optional_parameter_declaration,
        $.variadic_parameter_declaration,
      )),
      ')',
    ),

    requires_expression: $ => seq(
      'requires',
      field('parameters', optional(alias($.requires_parameter_list, $.parameter_list))),
      field('requirements', $.requirement_seq),
    ),

    lambda_specifier: $ => choice(
      'static',
      'constexpr',
      'consteval',
      'mutable',
    ),

    lambda_declarator: $ => choice(
      // main declarator form, includes parameter list
      seq(
        repeat($.attribute_declaration),
        field('parameters', $.parameter_list),
        repeat($.lambda_specifier),
        optional($._function_exception_specification),
        repeat($.attribute_declaration),
        optional($.trailing_return_type),
        optional($.requires_clause),
      ),

      // forms supporting omitted parameter list
      repeat1($.attribute_declaration),
      seq(
        repeat($.attribute_declaration),
        $.trailing_return_type,
      ),
      seq(
        repeat($.attribute_declaration),
        $._function_exception_specification,
        repeat($.attribute_declaration),
        optional($.trailing_return_type),
      ),
      seq(
        repeat($.attribute_declaration),
        repeat1($.lambda_specifier),
        optional($._function_exception_specification),
        repeat($.attribute_declaration),
        optional($.trailing_return_type),
      ),
    ),

    lambda_expression: $ => seq(
      field('captures', $.lambda_capture_specifier),
      optional(seq(
        field('template_parameters', $.template_parameter_list),
        optional(field('constraint', $.requires_clause)),
      )),
      optional(field('declarator', $.lambda_declarator)),
      field('body', $.compound_statement),
    ),

    lambda_capture_specifier: $ => prec(PREC.LAMBDA, seq(
      '[',
      choice(
        $.lambda_default_capture,
        commaSep($._lambda_capture),
        seq(
          $.lambda_default_capture,
          ',', commaSep1($._lambda_capture),
        ),
      ),
      ']',
    )),

    lambda_default_capture: _ => choice('=', '&'),

    _lambda_capture_identifier: $ => seq(
      optional('&'),
      choice(
        $.identifier,
        $.qualified_identifier,
        alias($.identifier_parameter_pack_expansion, $.parameter_pack_expansion),
      ),
    ),

    lambda_capture_initializer: $ => seq(
      optional('&'),
      optional('...'),
      field('left', $.identifier),
      '=',
      field('right', $.expression),
    ),

    _lambda_capture: $ => choice(
      seq(optional('*'), $.this),
      $._lambda_capture_identifier,
      $.lambda_capture_initializer,
    ),

    _fold_operator: _ => choice(...FOLD_OPERATORS),
    _binary_fold_operator: _ => choice(...FOLD_OPERATORS.map((operator) => seq(field('operator', operator), '...', operator))),

    _unary_left_fold: $ => seq(
      field('left', '...'),
      field('operator', $._fold_operator),
      field('right', $.expression),
    ),
    _unary_right_fold: $ => seq(
      field('left', $.expression),
      field('operator', $._fold_operator),
      field('right', '...'),
    ),
    _binary_fold: $ => seq(
      field('left', $.expression),
      $._binary_fold_operator,
      field('right', $.expression),
    ),

    fold_expression: $ => seq(
      '(',
      choice(
        $._unary_right_fold,
        $._unary_left_fold,
        $._binary_fold,
      ),
      ')',
    ),

    parameter_pack_expansion: $ => prec(-1, seq(
      field('pattern', $.expression),
      '...',
    )),

    type_parameter_pack_expansion: $ => seq(
      field('pattern', $.type_descriptor),
      '...',
    ),

    identifier_parameter_pack_expansion: $ => seq(
      field('pattern', $.identifier),
      '...',
    ),

    sizeof_expression: ($, original) => prec.right(PREC.SIZEOF, choice(
      original,
      seq(
        'sizeof', '...',
        '(',
        field('value', $.identifier),
        ')',
      ),
    )),

    unary_expression: ($, original) => choice(
      original,
      prec.left(PREC.UNARY, seq(
        field('operator', choice('not', 'compl')),
        field('argument', $.expression),
      )),
    ),

    binary_expression: ($, original) => {
      const table = [
        ['<=>', PREC.THREE_WAY],
        ['or', PREC.LOGICAL_OR],
        ['and', PREC.LOGICAL_AND],
        ['bitor', PREC.INCLUSIVE_OR],
        ['xor', PREC.EXCLUSIVE_OR],
        ['bitand', PREC.BITWISE_AND],
        ['not_eq', PREC.EQUAL],
      ];

      return choice(
        original,
        ...table.map(([operator, precedence]) => {
          return prec.left(precedence, seq(
            field('left', $.expression),
            // @ts-ignore
            field('operator', operator),
            field('right', $.expression),
          ));
        }));
    },

    // The compound_statement is added to parse macros taking statements as arguments, e.g. MYFORLOOP(1, 10, i, { foo(i); bar(i); })
    argument_list: $ => seq(
      '(',
      commaSep(choice($.expression, $.initializer_list, $.compound_statement)),
      ')',
    ),

    destructor_name: $ => prec(1, seq('~', $.identifier)),

    compound_literal_expression: ($, original) => choice(
      original,
      seq(
        field('type', choice(
          $._class_name,
          $.primitive_type,
          seq(optional('typename'), $.splice_type_specifier))),
        field('value', $.initializer_list),
      ),
    ),

    dependent_identifier: $ => seq('template', $.template_function),
    dependent_field_identifier: $ => seq('template', $.template_method),
    dependent_type_identifier: $ => seq('template', $.template_type),

    _scope_resolution: $ => prec(1, seq(
      field('scope', optional(choice(
        $._namespace_identifier,
        $.template_type,
        $.decltype,
        $.splice_expression,
        $.splice_type_specifier,
        alias($.dependent_type_identifier, $.dependent_name),
      ))),
      '::',
    )),

    qualified_field_identifier: $ => seq(
      $._scope_resolution,
      field('name', choice(
        alias($.dependent_field_identifier, $.dependent_name),
        alias($.qualified_field_identifier, $.qualified_identifier),
        $.template_method,
        prec.dynamic(2, $._field_identifier),
      )),
    ),

    qualified_identifier: $ => seq(
      $._scope_resolution,
      field('name', choice(
        alias($.dependent_identifier, $.dependent_name),
        $.qualified_identifier,
        $.template_function,
        prec.dynamic(1, seq(optional('template'), $.identifier)),
        $.operator_name,
        $.destructor_name,
        $.pointer_type_declarator,
      )),
    ),

    qualified_type_identifier: $ => seq(
      $._scope_resolution,
      field('name', choice(
        alias($.dependent_type_identifier, $.dependent_name),
        alias($.qualified_type_identifier, $.qualified_identifier),
        $.template_type,
        prec.dynamic(1, $._type_identifier),
      )),
    ),

    qualified_operator_cast_identifier: $ => seq(
      $._scope_resolution,
      field('name', choice(
        alias($.qualified_operator_cast_identifier, $.qualified_identifier),
        $.operator_cast,
      )),
    ),

    _assignment_left_expression: ($, original) => choice(
      original,
      $.qualified_identifier,
      $.user_defined_literal,
    ),

    assignment_expression: $ => prec.right(PREC.ASSIGNMENT, seq(
      field('left', $._assignment_left_expression),
      field('operator', choice(...ASSIGNMENT_OPERATORS)),
      field('right', choice($.expression, $.initializer_list)),
    )),

    _assignment_expression_lhs: $ => seq(
      field('left', $.expression),
      field('operator', choice(...ASSIGNMENT_OPERATORS)),
      field('right', choice($.expression, $.initializer_list)),
    ),

    // This prevents an ambiguity between fold expressions
    // and assignment expressions within parentheses.
    parenthesized_expression: ($, original) => choice(
      original,
      seq('(', alias($._assignment_expression_lhs, $.assignment_expression), ')'),
    ),

    reflect_expression: $ => prec.right(seq(
      '^^',
      choice(
        '::',
        $.expression,
        $.type_descriptor,
      ),
    )),

    splice_specifier: $ => seq( '[:', $.expression, ':]'),
    _splice_specialization_specifier: $ => seq($.splice_specifier, $.template_argument_list),

    splice_type_specifier: $ => prec.right(choice(
      $.splice_specifier,
      $._splice_specialization_specifier,
    )),

    splice_expression: $ => prec.right(choice(
      $.splice_specifier,
      seq('template', $._splice_specialization_specifier),
    )),

    expansion_statement: $ => seq(
      'template', 'for',
      '(',
      $._for_range_loop_body,
      ')',
      field('body', $.statement),
    ),

    operator_name: $ => prec(1, seq(
      'operator',
      choice(
        'co_await',
        '+', '-', '*', '/', '%',
        '^', '&', '|', '~',
        '!', '=', '<', '>',
        '+=', '-=', '*=', '/=', '%=', '^=', '&=', '|=',
        '<<', '>>', '>>=', '<<=',
        '==', '!=', '<=', '>=',
        '<=>',
        '&&', '||',
        '++', '--',
        ',',
        '->*',
        '->',
        '()', '[]',
        'xor', 'bitand', 'bitor', 'compl',
        'not', 'xor_eq', 'and_eq', 'or_eq', 'not_eq',
        'and', 'or',
        seq(choice('new', 'delete'), optional('[]')),
        seq('""', $.identifier),
      ),
    )),

    this: _ => 'this',

    concatenated_string: $ => prec.right(seq(
      choice($.identifier, $.string_literal, $.raw_string_literal),
      choice($.string_literal, $.raw_string_literal),
      repeat(choice($.identifier, $.string_literal, $.raw_string_literal)),
    )),

    number_literal: $ => {
      const sign = /[-\+]/;
      const separator = '\'';
      const binary = /[01]/;
      const binaryDigits = seq(repeat1(binary), repeat(seq(separator, repeat1(binary))));
      const decimal = /[0-9]/;
      const firstDecimal = /[1-9]/;
      const intDecimalDigits = seq(firstDecimal, repeat(decimal), repeat(seq(separator, repeat1(decimal))));
      const floatDecimalDigits = seq(repeat1(decimal), repeat(seq(separator, repeat1(decimal))));
      const hex = /[0-9a-fA-F]/;
      const hexDigits = seq(repeat1(hex), repeat(seq(separator, repeat1(hex))));
      const octal = /[0-7]/;
      const octalDigits = seq('0', repeat(octal), repeat(seq(separator, repeat1(octal))));
      const hexExponent = seq(/[pP]/, optional(sign), floatDecimalDigits);
      const decimalExponent = seq(/[eE]/, optional(sign), floatDecimalDigits);
      const intSuffix = /(ll|LL)[uU]?|[uU](ll|LL)?|[uU][lL]?|[uU][zZ]?|[lL][uU]?|[zZ][uU]?/;
      const floatSuffix = /([fF](16|32|64|128)?)|[lL]|(bf16|BF16)/;

      return token(seq(
        optional(sign),
        choice(
          seq(
            choice(
              seq(choice('0b', '0B'), binaryDigits),
              intDecimalDigits,
              seq(choice('0x', '0X'), hexDigits),
              octalDigits,
            ),
            optional(intSuffix),
          ),
          seq(
            choice(
              seq(floatDecimalDigits, decimalExponent),
              seq(floatDecimalDigits, '.', optional(floatDecimalDigits), optional(decimalExponent)),
              seq('.', floatDecimalDigits, optional(decimalExponent)),
              seq(
                choice('0x', '0X'),
                choice(
                  hexDigits,
                  seq(hexDigits, '.', optional(hexDigits)),
                  seq('.', hexDigits)),
                hexExponent,
              ),
            ),
            optional(floatSuffix),
          ),
        ),
      ));
    },

    literal_suffix: _ => token.immediate(/[a-zA-Z_]\w*/),

    user_defined_literal: $ => seq(
      choice(
        $.number_literal,
        $.char_literal,
        $._string,
      ),
      $.literal_suffix,
    ),

    _namespace_identifier: $ => alias($.identifier, $.namespace_identifier),



    // ▼▼▼ 追加: PRAGMA_ から始まる大文字のマクロを単独ステートメントとして許可 ▼▼▼
    unreal_pragma_macro: $ => token(prec(1, /PRAGMA_[A-Z0-9_]+/)),
    // ▲▲▲ 追加完了 ▲▲▲
    
    // --- START: UNREAL ENGINE RULES ---
    unreal_specifier_keyword: $ => choice(
     // UCLASS / USTRUCT / UENUM でよく使われるキーワード
      'Blueprintable',
      'BlueprintType',
      'Abstract',
      'abstract',
      'MinimalAPI',
      'Deprecated',

      // UPROPERTY
      'EditAnywhere',
      'EditDefaultsOnly',
      'EditInstanceOnly',
      'VisibleAnywhere',
      'VisibleDefaultsOnly',
      'VisibleInstanceOnly',
      'BlueprintReadOnly',
      'BlueprintReadWrite',
      'Config',
      'GlobalConfig',
      'Transient',
      'Replicated',
      'ReplicatedUsing',
      'SaveGame',

      // UFUNCTION 
      'BlueprintCallable',
      'BlueprintPure',
      'BlueprintImplementableEvent',
      'BlueprintNativeEvent',
      'Exec',
      'Server',
      'Client',
      'NetMulticast',
      'Reliable',
      'Unreliable',
      'WithValidation',

      //
      'Category',
      'meta',
      'DisplayName',
      'ToolTip'
    ),

    unreal_specifier: $ => choice(
      // key = value パターンを最優先
      prec(20, seq(
        field('key', choice($.unreal_specifier_keyword, $.identifier)),
        '=',
        field('value', $.unreal_specifier_content)
      )),
      // 単独の要素、あるいは複雑な式
      $.unreal_specifier_content
    ),

    unreal_specifier_content: $ => prec.left(repeat1(choice(
      $.string_literal,
      $.unreal_specifier_keyword,
      // 追加: Qualified identifiers (A::B) や テンプレート風 (A<B>) を一つの塊として扱う
      prec(10, seq($.identifier, repeat1(seq('::', $.identifier)))),
      prec(10, seq($.identifier, $.template_argument_list)),
      
      $.identifier,
      $.number_literal,
      $.true,
      $.false,
      '*', '&', 'const', // 追加: ポインタ、参照、const
      // ネストしたカッコ (TEXT(...) や meta=(...) など)
      seq('(', optional($.unreal_specifier_list), ')'),
      // フォールバック: カンマ、カッコ、等号、クォート以外の任意の文字列
      token(prec(-1, /[^,() \t\n="]+/))
    ))),

    _unreal_macro_content: $ => repeat1(choice(
      token(prec(10, /[^()]+/)),
      seq('(', optional($._unreal_macro_content), ')')
    )),

    unreal_argument_list: $ => seq(
      '(',
      optional($.unreal_specifier_list),
      ')'
    ),

    _unreal_macro_arguments: $ => $.unreal_argument_list,

    unreal_api_specifier: $ => token(prec(1, /[A-Z0-9_]+_API/)),

    uclass_macro: $ => seq('UCLASS', $.unreal_argument_list),
    ustruct_macro: $ => seq('USTRUCT', $.unreal_argument_list),
    uenum_macro: $ => seq('UENUM', $.unreal_argument_list),

    umeta_macro: $ => seq('UMETA', $.unreal_argument_list),

    primitive_type: (_, original) => choice(
      original,
      'int8', 'uint8', 'int16', 'uint16', 'int32', 'uint32', 'int64', 'uint64', 'FString', 'FName', 'FText'
    ),

    unreal_body_macro: $ => seq(
      choice('GENERATED_BODY', 'GENERATED_UCLASS_BODY', 'GENERATED_USTRUCT_BODY'),
      '(', ')'
    ),

    unreal_declare_class_macro: $ => seq(
      'DECLARE_CLASS',
      $.unreal_argument_list
    ),
    unreal_define_default_object_initializer_macro: $ => seq(
      'DEFINE_DEFAULT_OBJECT_INITIALIZER_CONSTRUCTOR_CALL',
      $.unreal_argument_list
    ),
    uproperty_macro: $ => seq('UPROPERTY', $.unreal_argument_list),
    ufunction_macro: $ => seq('UFUNCTION', $.unreal_argument_list),

    // Standalone rules for Unreal class/struct definitions with high precedence
    unreal_class_declaration: $ => prec(100, seq(
      $.uclass_macro,
      repeat($.comment),
      'class',
      optional($.unreal_api_specifier),
      field('name', $._class_name),
      optional($.base_class_clause),
      field('body', $.field_declaration_list),
      optional(';'),
    )),

    unreal_struct_declaration: $ => prec(100, seq(
      $.ustruct_macro,
      repeat($.comment),
      'struct',
      optional($.unreal_api_specifier),
      field('name', $._class_name),
      optional($.base_class_clause),
      field('body', $.field_declaration_list),
      optional(';'),
    )),

    unreal_enum_declaration: $ => prec(100, seq(
      $.uenum_macro,
      repeat($.comment),
      'enum',
      optional(choice('class', 'struct')),
      field('name', $._class_name),
      optional($._enum_base_clause),
      field('body', $.enumerator_list),
      optional(';')
    )),

    unreal_deprecated_macro: $ => prec(100, seq(
        choice('UE_DEPRECATED', 'UE_DEPRECATED_FORGAME'),
        $._unreal_macro_arguments
    )),

    // DECLARE_FUNCTION(...); ENUM_CLASS_FLAGS(...); などをキャッチ

    // 専用の緩い引数リスト定義
    unreal_specifier_list: $ => commaSep1($.unreal_specifier),

    // --- Unreal Delegate & Declaration Macros ---
    unreal_delegate_macro_name: $ => choice(
        'DECLARE_DELEGATE', 'DECLARE_DELEGATE_RetVal', 'DECLARE_DELEGATE_OneParam', 
        'DECLARE_DELEGATE_RetVal_OneParam', 'DECLARE_DELEGATE_TwoParams', 'DECLARE_DELEGATE_RetVal_TwoParams',
        'DECLARE_DELEGATE_ThreeParams', 'DECLARE_DELEGATE_RetVal_ThreeParams', 'DECLARE_DELEGATE_FourParams',
        'DECLARE_DELEGATE_RetVal_FourParams', 'DECLARE_DELEGATE_FiveParams', 'DECLARE_DELEGATE_RetVal_FiveParams',
        'DECLARE_MULTICAST_DELEGATE', 'DECLARE_MULTICAST_DELEGATE_OneParam', 'DECLARE_MULTICAST_DELEGATE_TwoParams',
        'DECLARE_DYNAMIC_DELEGATE', 'DECLARE_DYNAMIC_DELEGATE_RetVal', 'DECLARE_DYNAMIC_DELEGATE_OneParam',
        'DECLARE_DYNAMIC_MULTICAST_DELEGATE', 'DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam',
        'DECLARE_EVENT', 'DECLARE_TS_MULTICAST_DELEGATE'
    ),

    unreal_declaration_macro: $ => prec.left(3000, seq(
      field('api', optional($.unreal_api_specifier)),
      field('name', alias(
        token(prec(10, choice(
            'DECLARE_DELEGATE', 'DECLARE_DELEGATE_RetVal', 'DECLARE_DELEGATE_OneParam', 
            'DECLARE_DELEGATE_RetVal_OneParam', 'DECLARE_DELEGATE_TwoParams', 'DECLARE_DELEGATE_RetVal_TwoParams',
            'DECLARE_DELEGATE_ThreeParams', 'DECLARE_DELEGATE_RetVal_ThreeParams', 'DECLARE_DELEGATE_FourParams',
            'DECLARE_DELEGATE_RetVal_FourParams', 'DECLARE_DELEGATE_FiveParams', 'DECLARE_DELEGATE_RetVal_FiveParams',
            'DECLARE_DELEGATE_SixParams', 'DECLARE_DELEGATE_RetVal_SixParams',
            'DECLARE_DELEGATE_SevenParams', 'DECLARE_DELEGATE_RetVal_SevenParams',
            'DECLARE_DELEGATE_EightParams', 'DECLARE_DELEGATE_RetVal_EightParams',
            'DECLARE_DELEGATE_NineParams', 'DECLARE_DELEGATE_RetVal_NineParams',
            'DECLARE_MULTICAST_DELEGATE', 'DECLARE_MULTICAST_DELEGATE_OneParam', 'DECLARE_MULTICAST_DELEGATE_TwoParams',
            'DECLARE_DYNAMIC_DELEGATE', 'DECLARE_DYNAMIC_DELEGATE_RetVal', 'DECLARE_DYNAMIC_DELEGATE_OneParam',
            'DECLARE_DYNAMIC_MULTICAST_DELEGATE', 'DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam',
            'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE', 'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE_OneParam',
            'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE_TwoParams', 'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE_ThreeParams',
            'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE_FourParams', 'DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE_FiveParams',
            'DECLARE_EVENT', 'DECLARE_TS_MULTICAST_DELEGATE',
            'DEPRECATED_CHARACTER_MOVEMENT_RPC',
            /DECLARE_[A-Z0-9_]+/,
            'ENUM_CLASS_FLAGS',
            'IMPLEMENT_MODULE',
            'IMPLEMENT_GAME_MODULE',
            'IMPLEMENT_PRIMARY_GAME_MODULE'
        ))),
        $.unreal_macro_name
      )),
      field('arguments', $.unreal_argument_list),
      optional(';')
    )),
    unreal_force_inline: $ => token(prec(1, /FORCEINLINE(_[A-Z0-9_]+)?/)),
    // --- END: UNREAL ENGINE RULES ---

    //
    _pragma_argument: _ => token.immediate(prec(-1, /.*/)),
  },
});
