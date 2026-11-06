//! Inline calculator and number base conversion for the search box:
//! `1+2*3`, `sqrt(2)/2`, `0xff`, `255 to hex`, `0b1010 转十进制`.

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub value: String,
    pub label: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
enum Base {
    Dec,
    Hex,
    Bin,
    Oct,
}

impl Base {
    const ALL: [Base; 4] = [Base::Dec, Base::Hex, Base::Bin, Base::Oct];

    fn label(self) -> &'static str {
        match self {
            Base::Dec => "十进制",
            Base::Hex => "十六进制",
            Base::Bin => "二进制",
            Base::Oct => "八进制",
        }
    }

    fn format(self, n: i128) -> String {
        let (sign, n) = if n < 0 { ("-", n.unsigned_abs()) } else { ("", n as u128) };
        match self {
            Base::Dec => format!("{sign}{n}"),
            Base::Hex => format!("{sign}0x{n:X}"),
            Base::Bin => format!("{sign}0b{n:b}"),
            Base::Oct => format!("{sign}0o{n:o}"),
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        let name = name.trim().trim_end_matches("进制").trim();
        Some(match name {
            "dec" | "decimal" | "10" | "十" => Base::Dec,
            "hex" | "hexadecimal" | "16" | "十六" => Base::Hex,
            "bin" | "binary" | "2" | "二" => Base::Bin,
            "oct" | "octal" | "8" | "八" => Base::Oct,
            _ => return None,
        })
    }
}

pub fn evaluate(input: &str) -> Vec<Answer> {
    let input = input.trim();
    if input.is_empty() || input.len() > 200 {
        return Vec::new();
    }
    let lower = input.to_lowercase();
    let (expr, target) = split_target(&lower);
    let expr = expr.trim();
    let expr = expr.strip_prefix('=').unwrap_or(expr).trim();
    if expr.is_empty() {
        return Vec::new();
    }
    if let Some(n) = integer_literal(expr) {
        return match target {
            Some(base) => vec![Answer { value: base.format(n), label: base.label() }],
            None if expr.trim_start_matches('-').trim_start().get(..2).is_some_and(|p| matches!(p, "0x" | "0b" | "0o")) => {
                Base::ALL.iter().map(|b| Answer { value: b.format(n), label: b.label() }).collect()
            }
            None => Vec::new(),
        };
    }
    let mut parser = Parser { chars: normalize(expr).chars().collect(), pos: 0, operators: false, based: false };
    let Some(value) = parser.parse() else {
        return Vec::new();
    };
    if !value.is_finite() || !(parser.operators || target.is_some()) {
        return Vec::new();
    }
    let integer = (value.fract() == 0.0 && value.abs() < 9.0e15).then_some(value as i128);
    match (target, integer) {
        (Some(base), Some(n)) => vec![Answer { value: base.format(n), label: base.label() }],
        (Some(_), None) => Vec::new(),
        (None, _) => {
            let mut answers = vec![Answer { value: format_number(value), label: "计算结果" }];
            if let (Some(n), true) = (integer, parser.based) {
                answers.extend(Base::ALL[1..].iter().map(|b| Answer { value: b.format(n), label: b.label() }));
            }
            answers
        }
    }
}

fn split_target(input: &str) -> (&str, Option<Base>) {
    for sep in [" to ", " in ", "->", "=>", "转成", "转为", "转"] {
        if let Some(i) = input.rfind(sep) {
            if let Some(base) = Base::from_name(&input[i + sep.len()..]) {
                return (&input[..i], Some(base));
            }
        }
    }
    (input, None)
}

fn integer_literal(s: &str) -> Option<i128> {
    let s = s.replace('_', "");
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, s.as_str()),
    };
    let (radix, digits) = match digits.get(..2) {
        Some("0x") => (16, &digits[2..]),
        Some("0b") => (2, &digits[2..]),
        Some("0o") => (8, &digits[2..]),
        _ => (10, digits),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = i128::from_str_radix(digits, radix).ok()?;
    Some(if negative { -n } else { n })
}

fn normalize(s: &str) -> String {
    s.replace("**", "^")
        .replace(['×', '＊'], "*")
        .replace(['÷', '／'], "/")
        .replace('（', "(")
        .replace('）', ")")
        .replace('＋', "+")
        .replace(['－', '−'], "-")
        .replace('＾', "^")
        .replace('_', "")
}

fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i128);
    }
    if value.abs() >= 1e15 || value.abs() < 1e-9 {
        return format!("{value:e}");
    }
    let fixed = format!("{value:.12}");
    fixed.trim_end_matches('0').trim_end_matches('.').to_string()
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    operators: bool,
    based: bool,
}

impl Parser {
    fn parse(&mut self) -> Option<f64> {
        let value = self.expr()?;
        self.skip_space();
        (self.pos == self.chars.len()).then_some(value)
    }

    fn skip_space(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_space();
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.eat('+') {
                value += self.term()?;
            } else if self.eat('-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
            self.operators = true;
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.eat('*') {
                value *= self.unary()?;
            } else if self.eat('/') {
                value /= self.unary()?;
            } else if self.eat('%') {
                value %= self.unary()?;
            } else {
                return Some(value);
            }
            self.operators = true;
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.eat('^') {
            self.operators = true;
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn primary(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if c == '(' {
            self.pos += 1;
            let value = self.expr()?;
            return self.eat(')').then_some(value);
        }
        if c.is_ascii_digit() || c == '.' {
            return self.number();
        }
        if c == 'π' {
            self.pos += 1;
            return Some(std::f64::consts::PI);
        }
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_alphanumeric()) {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        match name.as_str() {
            "pi" => return Some(std::f64::consts::PI),
            "e" => return Some(std::f64::consts::E),
            "" => return None,
            _ => {}
        }
        if !self.eat('(') {
            return None;
        }
        let x = self.expr()?;
        if !self.eat(')') {
            return None;
        }
        self.operators = true;
        Some(match name.as_str() {
            "sqrt" => x.sqrt(),
            "abs" => x.abs(),
            "sin" => x.sin(),
            "cos" => x.cos(),
            "tan" => x.tan(),
            "asin" => x.asin(),
            "acos" => x.acos(),
            "atan" => x.atan(),
            "ln" => x.ln(),
            "log" | "lg" => x.log10(),
            "log2" => x.log2(),
            "exp" => x.exp(),
            "floor" => x.floor(),
            "ceil" => x.ceil(),
            "round" => x.round(),
            _ => return None,
        })
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        if self.chars.get(self.pos) == Some(&'0') {
            let radix = match self.chars.get(self.pos + 1) {
                Some('x') => 16,
                Some('b') => 2,
                Some('o') => 8,
                _ => 0,
            };
            if radix != 0 {
                self.pos += 2;
                let digits_start = self.pos;
                while self.chars.get(self.pos).is_some_and(|c| c.is_digit(radix)) {
                    self.pos += 1;
                }
                let digits: String = self.chars[digits_start..self.pos].iter().collect();
                self.based = true;
                return u64::from_str_radix(&digits, radix).ok().map(|n| n as f64);
            }
        }
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(input: &str) -> Option<String> {
        evaluate(input).into_iter().next().map(|a| a.value)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(first("1+2*3").as_deref(), Some("7"));
        assert_eq!(first("(1+2)*3").as_deref(), Some("9"));
        assert_eq!(first("=2^10").as_deref(), Some("1024"));
        assert_eq!(first("-2^2").as_deref(), Some("-4"));
        assert_eq!(first("2^-1").as_deref(), Some("0.5"));
        assert_eq!(first("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(first("10 % 4").as_deref(), Some("2"));
        assert_eq!(first("6×7").as_deref(), Some("42"));
        assert_eq!(first("（1＋2）÷4").as_deref(), Some("0.75"));
        assert_eq!(first("sqrt(16)").as_deref(), Some("4"));
        assert_eq!(first("pi*2").as_deref(), Some("6.28318530718"));
        assert_eq!(first("1/3").as_deref(), Some("0.333333333333"));
    }

    #[test]
    fn not_calculations() {
        for query in ["", "e", "pi", "42", "7-zip", "c++", "1/0", "win10", "notepad", "sqrt", "2(", "1 2"] {
            assert!(evaluate(query).is_empty(), "{query}");
        }
    }

    #[test]
    fn base_conversion() {
        let all = evaluate("0xff");
        assert_eq!(all.iter().map(|a| a.value.as_str()).collect::<Vec<_>>(), ["255", "0xFF", "0b11111111", "0o377"]);
        assert_eq!(first("255 to hex").as_deref(), Some("0xFF"));
        assert_eq!(first("255 in bin").as_deref(), Some("0b11111111"));
        assert_eq!(first("0b1010 转十进制").as_deref(), Some("10"));
        assert_eq!(first("0o17 -> dec").as_deref(), Some("15"));
        assert_eq!(first("255 转 16进制").as_deref(), Some("0xFF"));
        assert_eq!(first("-26 to hex").as_deref(), Some("-0x1A"));
        assert_eq!(first("0xffffffffffffffff to dec").as_deref(), Some("18446744073709551615"));
        assert_eq!(first("0x10 + 1").as_deref(), Some("17"));
        assert_eq!(evaluate("0x10 + 1")[1].value, "0x11");
        assert!(evaluate("1.5 to hex").is_empty());
    }
}
