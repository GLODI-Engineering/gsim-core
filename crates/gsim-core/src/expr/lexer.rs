//! Tokenizer for the `${{ ... }}` expression grammar. No lexer state escapes this module; a
//! `ParseError` here always carries the byte offset it failed at.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    String(String),
    Ident(String),
    True,
    False,
    Null,
    Dot,
    Comma,
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub message: String,
    pub at: usize,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '.' => {
                out.push(Token::Dot);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' => {
                out.push(Token::Star);
                i += 1;
            }
            '/' => {
                out.push(Token::Slash);
                i += 1;
            }
            '%' => {
                out.push(Token::Percent);
                i += 1;
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::Eq);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::Ne);
                i += 2;
            }
            '!' => {
                out.push(Token::Not);
                i += 1;
            }
            '<' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::Le);
                i += 2;
            }
            '<' => {
                out.push(Token::Lt);
                i += 1;
            }
            '>' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::Ge);
                i += 2;
            }
            '>' => {
                out.push(Token::Gt);
                i += 1;
            }
            '&' if chars.get(i + 1) == Some(&'&') => {
                out.push(Token::And);
                i += 2;
            }
            '|' if chars.get(i + 1) == Some(&'|') => {
                out.push(Token::Or);
                i += 2;
            }
            '\'' | '"' => {
                let quote = c;
                let start = i;
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(LexError {
                                message: "unterminated string literal".into(),
                                at: start,
                            });
                        }
                        Some(&q) if q == quote => {
                            i += 1;
                            break;
                        }
                        Some(&'\\') if chars.get(i + 1).is_some() => {
                            s.push(chars[i + 1]);
                            i += 2;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Token::String(s));
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let n: f64 = text.parse().map_err(|_| LexError {
                    message: format!("invalid number literal '{text}'"),
                    at: start,
                })?;
                out.push(Token::Number(n));
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(match text.as_str() {
                    "true" => Token::True,
                    "false" => Token::False,
                    "null" => Token::Null,
                    _ => Token::Ident(text),
                });
            }
            other => {
                return Err(LexError {
                    message: format!("unexpected character '{other}'"),
                    at: i,
                });
            }
        }
    }
    out.push(Token::End);
    Ok(out)
}
