use std::{fs, path::Path};

use boa_ast::{
    declaration::{
        Declaration as BoaDeclaration, ExportDeclaration as BoaExportDeclaration, ReExportKind,
    },
    expression::Expression as BoaExpression,
    scope::Scope,
    statement::Statement as BoaStatement,
};
use boa_interner::Interner;
use boa_parser::{
    Parser as BoaParser, Source, error::Error as BoaParseError, lexer::Error as BoaLexError,
};

use super::ast::{
    ExportAllDeclaration, ExportDefaultDeclaration, ExportNamedDeclaration, FunctionDeclaration,
    Program, SourceType, StatementNode, VariableDeclaration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl ParseError {
    #[must_use]
    pub fn new(message: impl Into<String>, line: usize, column: usize, offset: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
            offset,
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at line {}, col {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParserOptions {
    pub source_type: SourceType,
}

pub struct Parser<'a> {
    source: &'a str,
    options: ParserOptions,
}

impl<'a> Parser<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            options: ParserOptions::default(),
        }
    }

    #[must_use]
    pub fn with_options(source: &'a str, options: ParserOptions) -> Self {
        Self { source, options }
    }

    #[must_use]
    pub fn with_source_type(mut self, source_type: SourceType) -> Self {
        self.options.source_type = source_type;
        self
    }

    pub fn parse(&self) -> Result<Program, ParseError> {
        let mut interner = Interner::default();
        let mut parser = BoaParser::new(Source::from_bytes(self.source));

        match self.options.source_type {
            SourceType::Script => {
                let script = parser
                    .parse_script(&Scope::new_global(), &mut interner)
                    .map_err(|error| map_boa_error(self.source, error))?;
                let strict = script.strict();
                let body = script
                    .statements()
                    .statements()
                    .iter()
                    .cloned()
                    .map(statement_list_item_to_node)
                    .collect();
                Ok(Program::new(SourceType::Script, strict, body, interner))
            }
            SourceType::Module => {
                let module = parser
                    .parse_module(&Scope::new_global(), &mut interner)
                    .map_err(|error| map_boa_error(self.source, error))?;
                let body = module
                    .items()
                    .items()
                    .iter()
                    .cloned()
                    .map(module_item_to_node)
                    .collect();
                Ok(Program::new(SourceType::Module, true, body, interner))
            }
        }
    }

    pub fn parse_file(
        path: impl AsRef<Path>,
        source_type: SourceType,
    ) -> Result<Program, ParseError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|error| {
            ParseError::new(
                format!("failed to read {}: {error}", path.display()),
                1,
                1,
                0,
            )
        })?;
        Parser::new(&source).with_source_type(source_type).parse()
    }
}

fn module_item_to_node(item: boa_ast::ModuleItem) -> StatementNode {
    match item {
        boa_ast::ModuleItem::ImportDeclaration(import) => StatementNode::ImportDeclaration(import),
        boa_ast::ModuleItem::ExportDeclaration(export) => export_to_node(*export),
        boa_ast::ModuleItem::StatementListItem(item) => statement_list_item_to_node(item),
    }
}

fn statement_list_item_to_node(item: boa_ast::StatementListItem) -> StatementNode {
    match item {
        boa_ast::StatementListItem::Statement(statement) => statement_to_node(*statement),
        boa_ast::StatementListItem::Declaration(declaration) => declaration_to_node(*declaration),
    }
}

fn declaration_to_node(declaration: BoaDeclaration) -> StatementNode {
    match declaration {
        BoaDeclaration::FunctionDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::Function(function))
        }
        BoaDeclaration::GeneratorDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::Generator(function))
        }
        BoaDeclaration::AsyncFunctionDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::AsyncFunction(function))
        }
        BoaDeclaration::AsyncGeneratorDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::AsyncGenerator(function))
        }
        BoaDeclaration::ClassDeclaration(class_decl) => {
            StatementNode::ClassDeclaration(*class_decl)
        }
        BoaDeclaration::Lexical(lexical) => match lexical {
            boa_ast::declaration::LexicalDeclaration::Let(_) => {
                StatementNode::VariableDeclaration(VariableDeclaration::Let(lexical))
            }
            boa_ast::declaration::LexicalDeclaration::Const(_) => {
                StatementNode::VariableDeclaration(VariableDeclaration::Const(lexical))
            }
        },
    }
}

fn statement_to_node(statement: BoaStatement) -> StatementNode {
    match statement {
        BoaStatement::Block(block) => StatementNode::BlockStatement(block),
        BoaStatement::Var(var_decl) => {
            StatementNode::VariableDeclaration(VariableDeclaration::Var(var_decl))
        }
        BoaStatement::Empty => StatementNode::EmptyStatement,
        BoaStatement::Expression(expression) => match expression {
            BoaExpression::Debugger => StatementNode::DebuggerStatement,
            other => StatementNode::ExpressionStatement(other),
        },
        BoaStatement::If(if_statement) => StatementNode::IfStatement(if_statement),
        BoaStatement::DoWhileLoop(loop_statement) => {
            StatementNode::DoWhileStatement(loop_statement)
        }
        BoaStatement::WhileLoop(loop_statement) => StatementNode::WhileStatement(loop_statement),
        BoaStatement::ForLoop(loop_statement) => StatementNode::ForStatement(loop_statement),
        BoaStatement::ForInLoop(loop_statement) => StatementNode::ForInStatement(loop_statement),
        BoaStatement::ForOfLoop(loop_statement) => StatementNode::ForOfStatement(loop_statement),
        BoaStatement::Switch(switch_statement) => StatementNode::SwitchStatement(switch_statement),
        BoaStatement::Continue(statement) => StatementNode::ContinueStatement(statement),
        BoaStatement::Break(statement) => StatementNode::BreakStatement(statement),
        BoaStatement::Return(statement) => StatementNode::ReturnStatement(statement),
        BoaStatement::Labelled(statement) => StatementNode::LabeledStatement(statement),
        BoaStatement::Throw(statement) => StatementNode::ThrowStatement(statement),
        BoaStatement::Try(statement) => StatementNode::TryStatement(statement),
        BoaStatement::With(statement) => StatementNode::WithStatement(statement),
    }
}

fn export_to_node(export: BoaExportDeclaration) -> StatementNode {
    match &export {
        BoaExportDeclaration::ReExport {
            kind: ReExportKind::Namespaced { name: None },
            ..
        } => StatementNode::ExportAllDeclaration(ExportAllDeclaration(export)),
        BoaExportDeclaration::DefaultFunctionDeclaration(_)
        | BoaExportDeclaration::DefaultGeneratorDeclaration(_)
        | BoaExportDeclaration::DefaultAsyncFunctionDeclaration(_)
        | BoaExportDeclaration::DefaultAsyncGeneratorDeclaration(_)
        | BoaExportDeclaration::DefaultClassDeclaration(_)
        | BoaExportDeclaration::DefaultAssignmentExpression(_) => {
            StatementNode::ExportDefaultDeclaration(ExportDefaultDeclaration(export))
        }
        BoaExportDeclaration::ReExport { .. }
        | BoaExportDeclaration::List(_)
        | BoaExportDeclaration::VarStatement(_)
        | BoaExportDeclaration::Declaration(_) => {
            StatementNode::ExportNamedDeclaration(ExportNamedDeclaration(export))
        }
    }
}

fn map_boa_error(source: &str, error: BoaParseError) -> ParseError {
    match error {
        BoaParseError::Expected { found, span, .. } => ParseError::new(
            format!("unexpected token '{found}'"),
            to_usize(span.start().line_number()),
            to_usize(span.start().column_number()),
            offset_from_position(
                source,
                to_usize(span.start().line_number()),
                to_usize(span.start().column_number()),
            ),
        ),
        BoaParseError::Unexpected { message, span, .. } => ParseError::new(
            message.to_string(),
            to_usize(span.start().line_number()),
            to_usize(span.start().column_number()),
            offset_from_position(
                source,
                to_usize(span.start().line_number()),
                to_usize(span.start().column_number()),
            ),
        ),
        BoaParseError::General { message, position } => ParseError::new(
            message.to_string(),
            to_usize(position.line_number()),
            to_usize(position.column_number()),
            offset_from_position(
                source,
                to_usize(position.line_number()),
                to_usize(position.column_number()),
            ),
        ),
        BoaParseError::Lex { err } => map_lex_error(source, err),
        BoaParseError::AbruptEnd => {
            let (line, column) = last_position(source);
            ParseError::new("abrupt end of input", line, column, source.len())
        }
    }
}

fn map_lex_error(source: &str, error: BoaLexError) -> ParseError {
    match error {
        BoaLexError::Syntax(message, position) => ParseError::new(
            message.to_string(),
            to_usize(position.line_number()),
            to_usize(position.column_number()),
            offset_from_position(
                source,
                to_usize(position.line_number()),
                to_usize(position.column_number()),
            ),
        ),
        BoaLexError::IO(io_error) => ParseError::new(io_error.to_string(), 1, 1, 0),
    }
}

fn to_usize(value: u32) -> usize {
    value as usize
}

fn offset_from_position(source: &str, line: usize, column: usize) -> usize {
    let mut current_line = 1;
    let mut line_start = 0;

    for (index, ch) in source.char_indices() {
        if current_line == line {
            return line_start + column.saturating_sub(1);
        }
        if ch == '\n' {
            current_line += 1;
            line_start = index + 1;
        }
    }

    if current_line == line {
        return line_start + column.saturating_sub(1);
    }

    source.len()
}

fn last_position(source: &str) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;

    for ch in source.chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += ch.len_utf8();
        }
    }

    (line, column)
}

#[cfg(test)]
mod tests {
    use std::env;

    use crate::engine::ast::Program;

    use super::{Parser, SourceType, StatementNode};

    fn parse_script(source: &str) -> Program {
        Parser::new(source).parse().unwrap()
    }

    fn parse_module(source: &str) -> Program {
        Parser::new(source)
            .with_source_type(SourceType::Module)
            .parse()
            .unwrap()
    }

    #[test]
    fn parses_basic_expression_shapes() {
        let program = parse_script("foo.bar(1 + 2, baz?.qux ?? 0);");
        assert_eq!(program.body_len(), 1);
        assert!(matches!(
            program.body()[0],
            StatementNode::ExpressionStatement(_)
        ));
    }

    #[test]
    fn parses_functions_and_destructured_arrow_params() {
        let program = parse_script(
            "function demo({a, b = 1}, [c, ...rest]) { return a + c; }\nconst fnx = ({x}, [y = 2]) => x + y;",
        );
        assert_eq!(program.body_len(), 2);
        assert!(matches!(
            program.body()[0],
            StatementNode::FunctionDeclaration(_)
        ));
        assert!(matches!(
            program.body()[1],
            StatementNode::VariableDeclaration(_)
        ));
    }

    #[test]
    fn parses_class_extends_static_methods_and_private_fields() {
        let program = parse_script(
            "class Derived extends Base { static make() { return new this(); } #value = 1; get value() { return this.#value; } }",
        );
        assert_eq!(program.body_len(), 1);
        assert!(matches!(
            program.body()[0],
            StatementNode::ClassDeclaration(_)
        ));
    }

    #[test]
    fn parses_async_and_generator_forms() {
        let program = parse_script(
            "async function load() { await work(); }\nfunction* iter() { yield 1; }\nconst task = async () => await load();",
        );
        assert_eq!(program.body_len(), 3);
        assert!(matches!(
            program.body()[0],
            StatementNode::FunctionDeclaration(_)
        ));
        assert!(matches!(
            program.body()[1],
            StatementNode::FunctionDeclaration(_)
        ));
        assert!(matches!(
            program.body()[2],
            StatementNode::VariableDeclaration(_)
        ));
    }

    #[test]
    fn parses_destructuring_forms() {
        let program = parse_script(
            "let {a, b: {c = 1}} = obj;\nconst [x, , ...rest] = arr;\nfor (const {id, value} of list) { consume(id, value); }",
        );
        assert_eq!(program.body_len(), 3);
        assert!(matches!(
            program.body()[0],
            StatementNode::VariableDeclaration(_)
        ));
        assert!(matches!(
            program.body()[1],
            StatementNode::VariableDeclaration(_)
        ));
        assert!(matches!(
            program.body()[2],
            StatementNode::ForOfStatement(_)
        ));
    }

    #[test]
    fn parses_template_literals_and_tagged_templates() {
        let program = parse_script("tag`hello ${name + `${count}`}`;");
        assert_eq!(program.body_len(), 1);
        assert!(matches!(
            program.body()[0],
            StatementNode::ExpressionStatement(_)
        ));
    }

    #[test]
    fn parses_optional_chaining_and_nullish_coalescing() {
        let program = parse_script("value = maybe?.call?.(arg)?.prop ?? fallback;");
        assert_eq!(program.body_len(), 1);
        assert!(matches!(
            program.body()[0],
            StatementNode::ExpressionStatement(_)
        ));
    }

    #[test]
    fn parses_import_and_export_declarations() {
        let program = parse_module(
            "import value, { named as alias } from 'pkg';\nexport { alias };\nexport default class Demo {}\nexport * from 'other';",
        );
        assert!(program.body_len() >= 4);
        assert!(matches!(
            program.body()[0],
            StatementNode::ImportDeclaration(_)
        ));
        assert!(
            program
                .body()
                .iter()
                .any(|node| matches!(node, StatementNode::ExportNamedDeclaration(_)))
        );
        assert!(
            program
                .body()
                .iter()
                .any(|node| matches!(node, StatementNode::ExportDefaultDeclaration(_)))
        );
        assert!(
            program
                .body()
                .iter()
                .any(|node| matches!(node, StatementNode::ExportAllDeclaration(_)))
        );
    }

    #[test]
    fn parses_multi_kilobyte_bundle_like_snippet() {
        let mut source = String::from("'use strict';\n");
        for index in 0..220 {
            source.push_str(&format!(
                "const mod_{index} = (() => {{\n  const cache = new Map();\n  class Entry {{\n    #value = {index};\n    get value() {{ return this.#value; }}\n    run(input = {{ value: {index} }}) {{ return input?.value ?? this.#value; }}\n  }}\n  function build([first, ...rest], {{ extra = {index} }} = {{}}) {{\n    return tag`item ${{first ?? extra}} ${{rest.length}}`;\n  }}\n  return {{ Entry, build }};\n}})();\n"
            ));
        }

        let program = parse_script(&source);
        assert!(program.body_len() >= 220);
    }

    #[test]
    #[ignore = "manual large bundle verification"]
    fn parses_youtube_bundle_from_env_path() {
        let Ok(path) = env::var("TOBIRA_YOUTUBE_KEVLAR_BASE_PATH") else {
            return;
        };

        let program = super::Parser::parse_file(path, SourceType::Script).unwrap();
        assert!(program.body_len() > 0);
    }
}
