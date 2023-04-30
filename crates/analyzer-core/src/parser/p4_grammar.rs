use anyhow::Result;
use lazy_static::lazy_static;
use parking_lot::RwLock;

use super::*;
use crate::lexer::Token;

lazy_static! {
	static ref STRING_TO_TOKEN: HashMap<&'static str, Token> = ([
		("*", Token::Asterisk),
		("@", Token::AtSymbol),
		(",", Token::Comma),
		("(", Token::OpenParen),
		(")", Token::CloseParen),
	])
	.into();
}

macro_rules! rule_rhs {
	($lit:literal) => {
		{
			let lit: &'static str = $lit;
			// TODO: keep Arc's in the table
			Rule::Terminal(Rc::new(vec![STRING_TO_TOKEN[lit].clone()]))
		}
	};
	($name:ident | $($names:ident)|+) => {
		Rule::Choice(vec![$name, $($names),+])
	};
	($name:ident, $($names:ident),+) => {
		Rule::Sequence(vec![$name, $($names),+])
	};
	($name:ident rep) => {
		Rule::Repetition($name)
	};
	($name:ident) => {
		Rule::Sequence(vec![$name])
	};
	((Token::$name:ident)) => {
		// TODO: keep Rc's in a lookup table
		Rule::Terminal(Rc::new(vec![Token::$name]))
	};
	(()) => {
		Rule::Nothing
	};
	({$pat:pat $(if $cond:expr)?}) => {
		Rule::TerminalPredicate(|tk| match tk {
			$pat $(if $cond)? => true,
			_ => false,
		}, stringify!($pat $(if $cond:expr)?))
	};
}

macro_rules! grammar {
	($($name:ident =>
		$prefix:tt
		$(| $($or:tt)|+)?
		$(, $($seq:tt),+)?
		$($rep:ident)?
	);+$(;)?) => {
		[$(($name, rule_rhs!($prefix $(| $($or)|+)? $(, $($seq),+)? $($rep)?))),+]
	};
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum P4GrammarRules {
	annotation,
	annotations,
	at_symbol,
	close_paren,
	comma,
	dir_in,
	dir_inout,
	dir_out,
	direction,
	ident,
	maybe_annotation,
	maybe_comma,
	maybe_direction,
	nothing,
	open_paren,
	opt_type_params,
	p4program,
	parameter_comma,
	parameter_list,
	parameter_seq_rep,
	parameter_seq,
	parameter,
	parser_decl,
	parser_kw,
	start,
	top_level_decl,
	top_level_decls_end,
	top_level_decls_rep,
	top_level_decls,
	typ,
	whitespace,
	ws,
}

pub fn p4_parser() -> impl FnOnce(RwLock<Vec<Token>>) -> Parser<P4GrammarRules, Token> {
	use P4GrammarRules::*;

	let rules = grammar! {
		start => p4program;
		ws => whitespace rep;
		whitespace => (Token::Whitespace);

		p4program => ws, top_level_decls, ws;
		top_level_decls => top_level_decls_rep | top_level_decls_end | nothing;
		top_level_decls_rep => top_level_decl, ws, top_level_decls;
		top_level_decls_end => (Token::Semicolon);

		top_level_decl => parser_decl;
		annotations => annotation rep;
		annotation => at_symbol, ident;

		direction => dir_in | dir_out | dir_inout;
		dir_in    => { Token::Identifier(i) if i == "in" };
		dir_out   => { Token::Identifier(i) if i == "out" };
		dir_inout => { Token::Identifier(i) if i == "inout" };

		at_symbol => "@";
		comma => ",";
		close_paren => ")";
		open_paren => "(";
		ident => { Token::Identifier(_) };
		nothing => ();

		parser_kw => (Token::KwParser);
		parser_decl => annotations, ws, parser_kw, ws, ident, ws, opt_type_params, ws, parameter_list;

		parameter_list => open_paren, ws, parameter_seq, ws, close_paren;
		parameter_seq => parameter_seq_rep | parameter | nothing;
		parameter_seq_rep => parameter_comma, parameter_seq;
		parameter_comma => parameter, ws, comma;
		maybe_comma => comma | nothing;
		parameter => maybe_annotation, ws, maybe_direction, ws, typ, ws, ident;
		maybe_annotation => annotation | nothing;
		maybe_direction => direction | nothing;
		opt_type_params => nothing; // TODO: type params
		typ => ident; // TODO: full type syntax
	};

	Parser::from_rules(start, &rules).unwrap()
}

#[cfg(test)]
mod test {
	use super::{
		super::{ast::*, simplifier::simplify},
		*,
	};
	use pretty_assertions::{assert_eq, assert_ne};

	fn lex_str(s: &str) -> Vec<Token> {
		use crate::*;

		let db = Database::default();
		let buf = Buffer::new(&db, s.to_string());
		let file_id = FileId::new(&db, "foo.p4".to_string());
		let lexed = lex(&db, file_id, buf);
		lexed.lexemes(&db).iter().map(|(tk, _)| tk).cloned().collect()
	}

	#[test]
	fn basic() -> Result<()> {
		let mk_parser = p4_parser();
		let source = vec![
			Token::Whitespace,
			Token::KwParser,
			Token::Identifier("()".into()),
			Token::OpenParen,
			Token::CloseParen,
		];
		let source_lock = RwLock::new(source);
		let mut parser: Parser<P4GrammarRules, Token> = mk_parser(source_lock);

		let r = parser.parse();
		eprintln!("here it is {r:#?}");
		assert_eq!(r, Ok(ExistingMatch { cst: Cst::Repetition(vec![]), match_length: 0 }));

		Ok(())
	}

	#[test]
	fn with_lexer() -> Result<()> {
		let mk_parser = p4_parser();
		let stream = lex_str(
			r"
			parser test_parser(@annotation in type int_param);
		",
		);

		let source_lock = RwLock::new(stream);
		let mut parser = mk_parser(source_lock);

		let parsed = parser.parse();
		// assert_eq!(Err(ParserError::ExpectedEof), parsed);

		assert_eq!(
			simplify(parser.rules.clone(), parsed.unwrap()),
			P4Program {
				top_level_declarations: vec![TopLevelDeclaration {
					annotations: vec![],
					kind: TopLevelDeclarationKind::Parser(ParserDeclaration {
						parameters: ParameterList {
							list: vec![Parameter {
								annotations: vec![Annotation::Unknown("annotation".into())],
								direction: Some(Direction::In),
								typ: Type {
									name: Identifier { name: "type".to_string().into(), length: 1 },
									params: None
								},
								name: Identifier { name: "int_param".to_string().into(), length: 1 },
								length: 7
							}]
						}
					}),
					length: 13
				}]
			}
		);

		Ok(())
	}
}
