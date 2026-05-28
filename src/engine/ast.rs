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
pub type StatementListItemNode = StatementListItem;
pub type VariableDeclarator = Variable;
pub type BindingNode = Binding;
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
pub type TemplateElementNode = boa_ast::expression::literal::TemplateElement;
pub type LiteralNode = Literal;
pub type LiteralKindNode = boa_ast::expression::literal::LiteralKind;
pub type ExpressionNode = Expression;
pub type IdentifierNode = Identifier;
pub type FunctionBodyNode = boa_ast::function::FunctionBody;
pub type FormalParameterNode = boa_ast::function::FormalParameter;
pub type FormalParameterListNode = boa_ast::function::FormalParameterList;
pub type TaggedTemplateExpression = TaggedTemplate;
pub type RegexLiteral = RegExpLiteral;
pub type ArrayPatternNode = ArrayPattern;
pub type ObjectPatternNode = ObjectPattern;
pub type PatternNode = Pattern;
pub type AssignTargetNode = boa_ast::expression::operator::assign::AssignTarget;
pub type UpdateTargetNode = boa_ast::expression::operator::update::UpdateTarget;
pub type BinaryOpNode = boa_ast::expression::operator::binary::BinaryOp;
pub type ArithmeticOpNode = boa_ast::expression::operator::binary::ArithmeticOp;
pub type BitwiseOpNode = boa_ast::expression::operator::binary::BitwiseOp;
pub type RelationalOpNode = boa_ast::expression::operator::binary::RelationalOp;
pub type LogicalOpNode = boa_ast::expression::operator::binary::LogicalOp;
pub type AssignOpNode = boa_ast::expression::operator::assign::AssignOp;
pub type UnaryOpNode = boa_ast::expression::operator::unary::UnaryOp;
pub type UpdateOpNode = boa_ast::expression::operator::update::UpdateOp;
pub type PropertyAccessFieldNode = boa_ast::expression::access::PropertyAccessField;
pub type SimplePropertyAccessNode = boa_ast::expression::access::SimplePropertyAccess;
pub type PropertyNameNode = boa_ast::property::PropertyName;
pub type MethodDefinitionKindNode = boa_ast::property::MethodDefinitionKind;
pub type ObjectMethodDefinitionNode = boa_ast::expression::literal::ObjectMethodDefinition;
pub type ForLoopInitializerNode = boa_ast::statement::iteration::ForLoopInitializer;
pub type ForLoopInitializerLexicalNode = boa_ast::statement::iteration::ForLoopInitializerLexical;
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

    #[must_use]
    pub fn variables(&self) -> &[VariableDeclarator] {
        match self {
            Self::Var(value) => value.0.as_ref(),
            Self::Let(value) | Self::Const(value) => value.variable_list().as_ref(),
        }
    }

    #[must_use]
    pub const fn is_const(&self) -> bool {
        matches!(self, Self::Const(_))
    }

    #[must_use]
    pub const fn is_var(&self) -> bool {
        matches!(self, Self::Var(_))
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
    pub const fn name(&self) -> Identifier {
        match self {
            Self::Function(value) => value.name(),
            Self::Generator(value) => value.name(),
            Self::AsyncFunction(value) => value.name(),
            Self::AsyncGenerator(value) => value.name(),
        }
    }

    #[must_use]
    pub const fn parameters(&self) -> &FormalParameterListNode {
        match self {
            Self::Function(value) => value.parameters(),
            Self::Generator(value) => value.parameters(),
            Self::AsyncFunction(value) => value.parameters(),
            Self::AsyncGenerator(value) => value.parameters(),
        }
    }

    #[must_use]
    pub const fn body(&self) -> &FunctionBodyNode {
        match self {
            Self::Function(value) => value.body(),
            Self::Generator(value) => value.body(),
            Self::AsyncFunction(value) => value.body(),
            Self::AsyncGenerator(value) => value.body(),
        }
    }

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
    pub const fn name(&self) -> Option<Identifier> {
        match self {
            Self::Function(value) => value.name(),
            Self::Generator(value) => value.name(),
            Self::AsyncFunction(value) => value.name(),
            Self::AsyncGenerator(value) => value.name(),
        }
    }

    #[must_use]
    pub const fn parameters(&self) -> &FormalParameterListNode {
        match self {
            Self::Function(value) => value.parameters(),
            Self::Generator(value) => value.parameters(),
            Self::AsyncFunction(value) => value.parameters(),
            Self::AsyncGenerator(value) => value.parameters(),
        }
    }

    #[must_use]
    pub const fn body(&self) -> &FunctionBodyNode {
        match self {
            Self::Function(value) => value.body(),
            Self::Generator(value) => value.body(),
            Self::AsyncFunction(value) => value.body(),
            Self::AsyncGenerator(value) => value.body(),
        }
    }

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
    pub const fn name(&self) -> Option<Identifier> {
        match self {
            Self::Sync(value) => value.name(),
            Self::Async(value) => value.name(),
        }
    }

    #[must_use]
    pub const fn parameters(&self) -> &FormalParameterListNode {
        match self {
            Self::Sync(value) => value.parameters(),
            Self::Async(value) => value.parameters(),
        }
    }

    #[must_use]
    pub const fn body(&self) -> &FunctionBodyNode {
        match self {
            Self::Sync(value) => value.body(),
            Self::Async(value) => value.body(),
        }
    }

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

#[must_use]
pub fn statement_list_item_to_node(item: StatementListItemNode) -> StatementNode {
    match item {
        StatementListItemNode::Statement(statement) => statement_to_node(*statement),
        StatementListItemNode::Declaration(declaration) => declaration_to_node(*declaration),
    }
}

#[must_use]
pub fn declaration_to_node(declaration: boa_ast::declaration::Declaration) -> StatementNode {
    match declaration {
        boa_ast::declaration::Declaration::FunctionDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::Function(function))
        }
        boa_ast::declaration::Declaration::GeneratorDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::Generator(function))
        }
        boa_ast::declaration::Declaration::AsyncFunctionDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::AsyncFunction(function))
        }
        boa_ast::declaration::Declaration::AsyncGeneratorDeclaration(function) => {
            StatementNode::FunctionDeclaration(FunctionDeclaration::AsyncGenerator(function))
        }
        boa_ast::declaration::Declaration::ClassDeclaration(class_decl) => {
            StatementNode::ClassDeclaration(*class_decl)
        }
        boa_ast::declaration::Declaration::Lexical(lexical) => match lexical {
            boa_ast::declaration::LexicalDeclaration::Let(_) => {
                StatementNode::VariableDeclaration(VariableDeclaration::Let(lexical))
            }
            boa_ast::declaration::LexicalDeclaration::Const(_) => {
                StatementNode::VariableDeclaration(VariableDeclaration::Const(lexical))
            }
        },
    }
}

#[must_use]
pub fn statement_to_node(statement: Statement) -> StatementNode {
    match statement {
        Statement::Block(block) => StatementNode::BlockStatement(block),
        Statement::Var(var_decl) => {
            StatementNode::VariableDeclaration(VariableDeclaration::Var(var_decl))
        }
        Statement::Empty => StatementNode::EmptyStatement,
        Statement::Expression(expression) => match expression {
            Expression::Debugger => StatementNode::DebuggerStatement,
            other => StatementNode::ExpressionStatement(other),
        },
        Statement::If(if_statement) => StatementNode::IfStatement(if_statement),
        Statement::DoWhileLoop(loop_statement) => StatementNode::DoWhileStatement(loop_statement),
        Statement::WhileLoop(loop_statement) => StatementNode::WhileStatement(loop_statement),
        Statement::ForLoop(loop_statement) => StatementNode::ForStatement(loop_statement),
        Statement::ForInLoop(loop_statement) => StatementNode::ForInStatement(loop_statement),
        Statement::ForOfLoop(loop_statement) => StatementNode::ForOfStatement(loop_statement),
        Statement::Switch(switch_statement) => StatementNode::SwitchStatement(switch_statement),
        Statement::Continue(statement) => StatementNode::ContinueStatement(statement),
        Statement::Break(statement) => StatementNode::BreakStatement(statement),
        Statement::Return(statement) => StatementNode::ReturnStatement(statement),
        Statement::Labelled(statement) => StatementNode::LabeledStatement(statement),
        Statement::Throw(statement) => StatementNode::ThrowStatement(statement),
        Statement::Try(statement) => StatementNode::TryStatement(statement),
        Statement::With(statement) => StatementNode::WithStatement(statement),
    }
}
