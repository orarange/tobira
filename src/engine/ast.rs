use boa_ast::{
    Keyword, Module, Position, Script, Span, StatementListItem,
    declaration::{
        Binding, ExportDeclaration as BoaExportDeclaration, ImportDeclaration, LexicalDeclaration,
        VarDeclaration, Variable,
    },
    expression::{
        Await, Call, Expression, Identifier, ImportCall, ImportMeta, New, NewTarget, Optional,
        RegExpLiteral, Spread, TaggedTemplate, Yield,
        access::PropertyAccess,
        literal::{ArrayLiteral, Literal, ObjectLiteral, TemplateLiteral},
        operator::{Assign, Binary, Conditional, Unary, Update},
    },
    function::{
        ArrowFunction, AsyncArrowFunction, AsyncFunctionDeclaration, AsyncFunctionExpression,
        AsyncGeneratorDeclaration, AsyncGeneratorExpression, ClassDeclaration, ClassElement,
        ClassExpression, FunctionDeclaration as BoaFunctionDeclaration,
        FunctionExpression as BoaFunctionExpression, GeneratorDeclaration, GeneratorExpression,
        PrivateName,
    },
    pattern::{ArrayPattern, ObjectPattern, Pattern},
    statement::{
        Block, Case, Catch, If, Labelled, Return, Switch, Throw, Try, With,
        iteration::{Break, Continue, DoWhileLoop, ForInLoop, ForLoop, ForOfLoop, WhileLoop},
    },
};
use boa_interner::{Interner, Sym};

pub type SourcePosition = Position;
pub type SourceSpan = Span;
pub type Statement = boa_ast::statement::Statement;
pub type VariableDeclarator = Variable;
pub type SwitchCase = Case;
pub type CatchClause = Catch;
pub type BlockStatement = Block;
pub type IfStatement = If;
pub type SwitchStatement = Switch;
pub type ForStatement = ForLoop;
pub type ForInStatement = ForInLoop;
pub type ForOfStatement = ForOfLoop;
pub type WhileStatement = WhileLoop;
pub type DoWhileStatement = DoWhileLoop;
pub type TryStatement = Try;
pub type ThrowStatement = Throw;
pub type ReturnStatement = Return;
pub type BreakStatement = Break;
pub type ContinueStatement = Continue;
pub type LabeledStatement = Labelled;
pub type JSImportDeclaration = ImportDeclaration;
pub type JSImportExpression = ImportCall;
pub type JSImportMeta = ImportMeta;
pub type JSNewTarget = NewTarget;
pub type ArrayExpression = ArrayLiteral;
pub type ObjectExpression = ObjectLiteral;
pub type MemberExpression = PropertyAccess;
pub type OptionalMemberExpression = Optional;
pub type OptionalCallExpression = Optional;
pub type CallExpression = Call;
pub type NewExpression = New;
pub type UnaryExpression = Unary;
pub type BinaryExpression = Binary;
pub type LogicalExpression = Binary;
pub type AssignmentExpression = Assign;
pub type UpdateExpression = Update;
pub type ConditionalExpression = Conditional;
pub type SpreadElement = Spread;
pub type AwaitExpression = Await;
pub type YieldExpression = Yield;
pub type TemplateLiteralExpression = TemplateLiteral;
pub type TaggedTemplateExpression = TaggedTemplate;
pub type RegexLiteral = RegExpLiteral;
pub type ArrayPatternNode = ArrayPattern;
pub type ObjectPatternNode = ObjectPattern;
pub type PatternNode = Pattern;
pub type PrivateIdentifier = PrivateName;
pub type ClassBody = Box<[ClassElement]>;
pub type MethodDefinition = ClassElement;
pub type PropertyDefinition = ClassElement;
pub type ObjectPropertyDefinition = boa_ast::expression::literal::PropertyDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceType {
    #[default]
    Script,
    Module,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProgramKind {
    Script(Script),
    Module(Module),
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariableDeclaration {
    Var(VarDeclaration),
    Let(LexicalDeclaration),
    Const(LexicalDeclaration),
}

impl VariableDeclaration {
    #[must_use]
    pub const fn as_var(&self) -> Option<&VarDeclaration> {
        match self {
            Self::Var(value) => Some(value),
            Self::Let(_) | Self::Const(_) => None,
        }
    }

    #[must_use]
    pub const fn as_lexical(&self) -> Option<&LexicalDeclaration> {
        match self {
            Self::Var(_) => None,
            Self::Let(value) | Self::Const(value) => Some(value),
        }
    }

    #[must_use]
    pub const fn keyword(&self) -> &'static str {
        match self {
            Self::Var(_) => "var",
            Self::Let(_) => "let",
            Self::Const(_) => "const",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionDeclaration {
    Function(BoaFunctionDeclaration),
    Generator(GeneratorDeclaration),
    AsyncFunction(AsyncFunctionDeclaration),
    AsyncGenerator(AsyncGeneratorDeclaration),
}

impl FunctionDeclaration {
    #[must_use]
    pub const fn is_async(&self) -> bool {
        matches!(self, Self::AsyncFunction(_) | Self::AsyncGenerator(_))
    }

    #[must_use]
    pub const fn is_generator(&self) -> bool {
        matches!(self, Self::Generator(_) | Self::AsyncGenerator(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionExpression {
    Function(BoaFunctionExpression),
    Generator(GeneratorExpression),
    AsyncFunction(AsyncFunctionExpression),
    AsyncGenerator(AsyncGeneratorExpression),
}

impl FunctionExpression {
    #[must_use]
    pub const fn is_async(&self) -> bool {
        matches!(self, Self::AsyncFunction(_) | Self::AsyncGenerator(_))
    }

    #[must_use]
    pub const fn is_generator(&self) -> bool {
        matches!(self, Self::Generator(_) | Self::AsyncGenerator(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrowFunctionExpression {
    Sync(ArrowFunction),
    Async(AsyncArrowFunction),
}

impl ArrowFunctionExpression {
    #[must_use]
    pub const fn is_async(&self) -> bool {
        matches!(self, Self::Async(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetaProperty {
    ImportMeta(ImportMeta),
    NewTarget(NewTarget),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportNamedDeclaration(pub BoaExportDeclaration);

#[derive(Debug, Clone, PartialEq)]
pub struct ExportDefaultDeclaration(pub BoaExportDeclaration);

#[derive(Debug, Clone, PartialEq)]
pub struct ExportAllDeclaration(pub BoaExportDeclaration);

#[derive(Debug, Clone, PartialEq)]
pub enum StatementNode {
    VariableDeclaration(VariableDeclaration),
    FunctionDeclaration(FunctionDeclaration),
    ClassDeclaration(ClassDeclaration),
    BlockStatement(BlockStatement),
    IfStatement(IfStatement),
    SwitchStatement(SwitchStatement),
    ForStatement(ForStatement),
    ForInStatement(ForInStatement),
    ForOfStatement(ForOfStatement),
    WhileStatement(WhileStatement),
    DoWhileStatement(DoWhileStatement),
    TryStatement(TryStatement),
    ThrowStatement(ThrowStatement),
    ReturnStatement(ReturnStatement),
    BreakStatement(BreakStatement),
    ContinueStatement(ContinueStatement),
    LabeledStatement(LabeledStatement),
    ExpressionStatement(Expression),
    EmptyStatement,
    ImportDeclaration(JSImportDeclaration),
    ExportNamedDeclaration(ExportNamedDeclaration),
    ExportDefaultDeclaration(ExportDefaultDeclaration),
    ExportAllDeclaration(ExportAllDeclaration),
    DebuggerStatement,
    WithStatement(With),
}

#[derive(Debug)]
pub struct Program {
    source_type: SourceType,
    strict: bool,
    body: Vec<StatementNode>,
    interner: Interner,
}

impl Program {
    #[must_use]
    pub fn new(
        source_type: SourceType,
        strict: bool,
        body: Vec<StatementNode>,
        interner: Interner,
    ) -> Self {
        Self {
            source_type,
            strict,
            body,
            interner,
        }
    }

    #[must_use]
    pub const fn source_type(&self) -> SourceType {
        self.source_type
    }

    #[must_use]
    pub const fn strict(&self) -> bool {
        self.strict
    }

    #[must_use]
    pub fn body(&self) -> &[StatementNode] {
        &self.body
    }

    #[must_use]
    pub fn body_len(&self) -> usize {
        self.body.len()
    }

    #[must_use]
    pub const fn interner(&self) -> &Interner {
        &self.interner
    }

    #[must_use]
    pub fn resolve_sym(&self, sym: Sym) -> String {
        self.interner.resolve_expect(sym).to_string()
    }
}
