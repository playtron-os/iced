//! Inline TeX math, set as plain Unicode text.
//!
//! Language models write `$\rightarrow$`, `$x^2$` and `$5 \times 3$` in prose.
//! Without a TeX engine the best reading of those is the symbol itself: the
//! arrows, operators and Greek letters, degrees, and simple super- and
//! subscripts. A command this module does not know stays as its source.
use std::borrow::Cow;

/// `text` with its inline math set as Unicode, and everything else as it was.
///
/// Math is `$…$`, `$$…$$`, `\(…\)` or `\[…\]`. A single `$` follows Pandoc's
/// rule: it opens before a non-space, and closes after a non-space when no
/// digit follows, so "$5 and $10" and "$5-$10" stay money. Math holds no `$`
/// and never crosses a backtick, and a code span is left as it is.
pub fn render_inline(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;

    while i < bytes.len() {
        let found = match bytes[i] {
            b'`' => {
                i = past_code(text, i);
                continue;
            }
            b'\\' => match bytes.get(i + 1) {
                Some(b'(') => bracketed(text, i, "\\)"),
                Some(b'[') => bracketed(text, i, "\\]"),
                // An escaped character, such as `\$`, is never a delimiter.
                Some(next) if next.is_ascii() => {
                    i += 2;
                    continue;
                }
                _ => None,
            },
            b'$' if bytes.get(i + 1) == Some(&b'$') => bracketed(text, i, "$$"),
            b'$' => dollars(text, i),
            _ => None,
        };

        match found {
            Some((tex, end)) => {
                out.push_str(&text[copied..i]);
                out.push_str(&to_unicode(tex));
                copied = end;
                i = end;
            }
            None => i += 1,
        }
    }

    if copied == 0 {
        Cow::Borrowed(text)
    } else {
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    }
}

/// Where the code span opening at `start` ends, or just past its backticks
/// when it never closes.
fn past_code(text: &str, start: usize) -> usize {
    let run = text[start..].bytes().take_while(|&b| b == b'`').count();
    let fence = &text[start..start + run];
    let mut at = start + run;

    while let Some(offset) = text[at..].find(fence) {
        let close = at + offset;
        let len = text[close..].bytes().take_while(|&b| b == b'`').count();
        if len == run {
            return close + run;
        }
        at = close + len;
    }

    start + run
}

/// The math between the two-character opener at `start` and `close`, and
/// where it ends.
fn bracketed<'a>(text: &'a str, start: usize, close: &str) -> Option<(&'a str, usize)> {
    let body = start + 2;
    let end = text[body..].find(close)? + body;
    let tex = text[body..end].trim();

    if tex.is_empty() || tex.contains('`') {
        return None;
    }

    Some((tex, end + close.len()))
}

/// The math in a `$…$` opening at `start`, by Pandoc's rule.
fn dollars(text: &str, start: usize) -> Option<(&str, usize)> {
    let body = start + 1;
    if text[body..].starts_with(char::is_whitespace) || body == text.len() {
        return None;
    }

    let mut at = body;
    let close = loop {
        let offset = text[at..].find('$')?;
        let close = at + offset;
        // An escaped dollar is part of the math.
        if text[..close].ends_with('\\') {
            at = close + 1;
            continue;
        }
        break close;
    };

    let tex = &text[body..close];
    let closes = !tex.ends_with(char::is_whitespace)
        && !text[close + 1..].starts_with(|c: char| c.is_ascii_digit());

    if !closes || tex.contains('`') {
        return None;
    }

    Some((tex, close + 1))
}

/// TeX math as Unicode text: arrows, operators, Greek letters, degrees,
/// fractions, roots, and the super- and subscripts Unicode has.
///
/// Spaces stay where they were written, since the result is read as prose.
/// A command with no Unicode form stays as its source, braces and all.
pub fn to_unicode(tex: &str) -> String {
    let mut out = String::new();
    Reader { tex, at: 0 }.convert(&mut out, false);

    collapse_spaces(&out)
}

/// One pass over TeX source.
struct Reader<'a> {
    tex: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.tex[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            let _ = self.bump();
        }
    }

    /// Everything up to the end, or, when `nested`, to the `}` closing the
    /// group it is in.
    fn convert(&mut self, out: &mut String, nested: bool) {
        while let Some(c) = self.peek() {
            match c {
                '}' if nested => return,
                '{' => {
                    let group = self.group();
                    out.push_str(&group);
                }
                '\\' => self.command(out),
                '^' | '_' => {
                    let _ = self.bump();
                    let argument = self.argument();
                    out.push_str(&script(&argument, c == '^'));
                }
                '~' => {
                    let _ = self.bump();
                    out.push(' ');
                }
                _ => {
                    let _ = self.bump();
                    out.push(c);
                }
            }
        }
    }

    /// A `{…}` group at the cursor, converted, braces dropped.
    fn group(&mut self) -> String {
        let _ = self.bump();
        let mut inner = String::new();
        self.convert(&mut inner, true);
        let _ = self.bump();
        inner
    }

    /// Verbatim text in a `{…}` group, for `\text` and its kin.
    fn text_group(&mut self) -> String {
        let _ = self.bump();
        let mut depth = 0_usize;
        let mut inner = String::new();

        while let Some(c) = self.bump() {
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                // `\%`, `\$`, `\&`: a character escaped in text mode.
                '\\' => match self.peek() {
                    Some(next) if !next.is_ascii_alphabetic() => {
                        let _ = self.bump();
                        inner.push(next);
                    }
                    _ => inner.push(c),
                },
                _ => inner.push(c),
            }
        }

        inner
    }

    /// A command's or a script's argument: a group, a command, or one
    /// character.
    fn argument(&mut self) -> String {
        self.skip_spaces();
        match self.peek() {
            Some('{') => self.group(),
            Some('\\') => {
                let mut out = String::new();
                self.command(&mut out);
                out
            }
            Some(_) => self.bump().map(String::from).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// A command at the cursor, written to `out`.
    fn command(&mut self, out: &mut String) {
        let start = self.at;
        let _ = self.bump();

        let name_len = self.tex[self.at..]
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .count();

        if name_len == 0 {
            // A control symbol: `\,`, `\%`, `\{`, `\\`.
            let Some(c) = self.bump() else {
                out.push('\\');
                return;
            };
            match c {
                ',' | ';' | ':' | ' ' | '>' | '\\' => out.push(' '),
                '!' => {}
                '|' => out.push('‖'),
                _ => out.push(c),
            }
            return;
        }

        let name = &self.tex[self.at..self.at + name_len];
        self.at += name_len;

        if let Some(symbol) = symbol(name) {
            out.push_str(symbol);
            return;
        }

        match name {
            "text" | "textrm" | "textnormal" | "textit" | "textbf" | "textsf" | "texttt"
            | "emph" | "mbox" | "hbox" | "operatorname" => {
                self.skip_spaces();
                if self.peek() == Some('{') {
                    out.push_str(&self.text_group());
                }
            }
            "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "mathcal" | "mathfrak"
            | "mathscr" | "boldsymbol" | "bm" => {
                out.push_str(&self.argument());
            }
            "mathbb" => {
                let letter = self.argument();
                out.push_str(match letter.as_str() {
                    "N" => "ℕ",
                    "Z" => "ℤ",
                    "Q" => "ℚ",
                    "R" => "ℝ",
                    "C" => "ℂ",
                    _ => &letter,
                });
            }
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let numerator = self.argument();
                let denominator = self.argument();
                out.push_str(&operand(&numerator));
                out.push('/');
                out.push_str(&operand(&denominator));
            }
            "sqrt" => {
                self.skip_spaces();
                let mut root = "√";
                if self.peek() == Some('[') {
                    let close = self.tex[self.at..].find(']').map(|i| self.at + i);
                    if let Some(close) = close {
                        root = match self.tex[self.at + 1..close].trim() {
                            "3" => "∛",
                            "4" => "∜",
                            _ => "√",
                        };
                        self.at = close + 1;
                    }
                }
                let radicand = self.argument();
                out.push_str(root);
                out.push_str(&operand(&radicand));
            }
            "left" | "right" | "middle" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr"
            | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" => {
                // `\left.` is an invisible delimiter.
                if self.peek() == Some('.') {
                    let _ = self.bump();
                }
            }
            "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" => {}
            _ => {
                // Unknown: its source, with the groups it takes.
                out.push_str(&self.tex[start..self.at]);
                while self.peek() == Some('{') {
                    out.push('{');
                    out.push_str(&self.group());
                    out.push('}');
                }
            }
        }
    }
}

/// A fraction's or a root's operand, in parentheses unless it is one word or
/// number.
fn operand(text: &str) -> String {
    let text = text.trim();
    if text.chars().all(|c| c.is_alphanumeric() || c == '.') {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

/// `^…` or `_…` in Unicode's super- or subscript characters, or written out
/// when Unicode lacks one of them.
fn script(argument: &str, superscript: bool) -> String {
    let argument = argument.trim();
    // `^\circ` is how TeX writes a degree.
    if superscript && argument == "∘" {
        return String::from("°");
    }

    let mapped: Option<String> = argument
        .chars()
        .map(|c| {
            if superscript {
                superscript_of(c)
            } else {
                subscript_of(c)
            }
        })
        .collect();

    match mapped {
        Some(mapped) if !mapped.is_empty() => mapped,
        _ => {
            let mark = if superscript { '^' } else { '_' };
            if argument.chars().count() == 1 {
                format!("{mark}{argument}")
            } else {
                format!("{mark}({argument})")
            }
        }
    }
}

fn superscript_of(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        _ => return None,
    })
}

fn subscript_of(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        _ => return None,
    })
}

/// Runs of white space as one space, and none at either end.
fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Unicode a command with no argument stands for.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Arrows
        "rightarrow" | "to" => "→",
        "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" | "impliedby" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "longleftrightarrow" => "⟷",
        "Longrightarrow" => "⟹",
        "Longleftarrow" => "⟸",
        "Longleftrightarrow" => "⟺",
        "mapsto" => "↦",
        "longmapsto" => "⟼",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "updownarrow" => "↕",
        "Uparrow" => "⇑",
        "Downarrow" => "⇓",
        "nearrow" => "↗",
        "searrow" => "↘",
        "swarrow" => "↙",
        "nwarrow" => "↖",
        "hookrightarrow" => "↪",
        "hookleftarrow" => "↩",
        "leadsto" | "rightsquigarrow" => "↝",
        "rightleftharpoons" => "⇌",
        // Operators and relations
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "•",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "setminus" => "∖",
        "le" | "leq" | "leqslant" => "≤",
        "ge" | "geq" | "geqslant" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "lt" => "<",
        "gt" => ">",
        "mid" | "vert" | "lvert" | "rvert" => "|",
        "Vert" | "lVert" | "rVert" | "parallel" => "‖",
        "perp" | "bot" => "⊥",
        "top" => "⊤",
        "angle" => "∠",
        "triangle" => "△",
        // Sets and logic
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" | "varnothing" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "therefore" => "∴",
        "because" => "∵",
        // Calculus
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "oint" => "∮",
        "prime" => "′",
        // Dots, brackets and spaces
        "ldots" | "dots" | "cdots" | "dotsc" | "dotsb" => "…",
        "vdots" => "⋮",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lbrace" => "{",
        "rbrace" => "}",
        "quad" | "qquad" | "space" | "enspace" | "thinspace" => " ",
        // Letters and marks
        "degree" => "°",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "checkmark" => "✓",
        "dagger" => "†",
        "ddagger" => "‡",
        "S" => "§",
        "P" => "¶",
        "copyright" => "©",
        "pounds" => "£",
        "euro" => "€",
        "yen" => "¥",
        // Function names, set upright in TeX
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "cot" => "cot",
        "sec" => "sec",
        "csc" => "csc",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "exp" => "exp",
        "lim" => "lim",
        "max" => "max",
        "min" => "min",
        "sup" => "sup",
        "inf" => "inf",
        "det" => "det",
        "gcd" => "gcd",
        "deg" => "deg",
        "arg" => "arg",
        "dim" => "dim",
        "ker" => "ker",
        "mod" | "bmod" => "mod",
        // Greek
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "omicron" => "ο",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_become_arrows() {
        assert_eq!(
            render_inline(r"Order Received $\rightarrow$ In Queue $\rightarrow$ Printing"),
            "Order Received → In Queue → Printing"
        );
        assert_eq!(render_inline(r"$a \to b$"), "a → b");
        assert_eq!(render_inline(r"$\leftarrow$"), "←");
        assert_eq!(render_inline(r"$\Rightarrow$"), "⇒");
        assert_eq!(render_inline(r"$\leftrightarrow$"), "↔");
        assert_eq!(render_inline(r"$x \mapsto 2x$"), "x ↦ 2x");
    }

    #[test]
    fn operators_and_relations() {
        assert_eq!(render_inline(r"$3 \times 4$"), "3 × 4");
        assert_eq!(render_inline(r"$a \cdot b$"), "a · b");
        assert_eq!(render_inline(r"$\pm 2$"), "± 2");
        assert_eq!(render_inline(r"$8 \div 2$"), "8 ÷ 2");
        assert_eq!(render_inline(r"$x \le 5$, $y \geq 1$"), "x ≤ 5, y ≥ 1");
        assert_eq!(render_inline(r"$a \neq b$"), "a ≠ b");
        assert_eq!(render_inline(r"$\pi \approx 3.14$"), "π ≈ 3.14");
        assert_eq!(render_inline(r"$a \equiv b$"), "a ≡ b");
        assert_eq!(render_inline(r"$\infty$"), "∞");
        assert_eq!(render_inline(r"$a \sim b$"), "a ∼ b");
    }

    #[test]
    fn greek_letters() {
        assert_eq!(render_inline(r"$\alpha + \beta = \Gamma$"), "α + β = Γ");
        assert_eq!(render_inline(r"$\Delta T$"), "Δ T");
    }

    #[test]
    fn degrees() {
        assert_eq!(render_inline(r"$200^\circ$C"), "200°C");
        assert_eq!(render_inline(r"$200^{\circ}\text{C}$"), "200°C");
        assert_eq!(render_inline(r"$45\degree$"), "45°");
    }

    #[test]
    fn simple_scripts() {
        assert_eq!(render_inline(r"$x^2$"), "x²");
        assert_eq!(render_inline(r"$H_2O$"), "H₂O");
        assert_eq!(render_inline(r"$10^{-3}$"), "10⁻³");
        assert_eq!(render_inline(r"$x_{12}$"), "x₁₂");
        // No superscript `y` in Unicode: written out instead.
        assert_eq!(render_inline(r"$e^{y+1}$"), "e^(y+1)");
        assert_eq!(render_inline(r"$a_b$"), "a_b");
    }

    #[test]
    fn fractions_roots_and_text() {
        assert_eq!(render_inline(r"$\frac{1}{2}$"), "1/2");
        assert_eq!(render_inline(r"$\frac{a+b}{2}$"), "(a+b)/2");
        assert_eq!(render_inline(r"$\sqrt{2}$"), "√2");
        assert_eq!(render_inline(r"$\sqrt{x+1}$"), "√(x+1)");
        assert_eq!(render_inline(r"$5\text{ mm}$"), "5 mm");
        assert_eq!(render_inline(r"$\mathbb{R}$"), "ℝ");
        assert_eq!(render_inline(r"$50\%$"), "50%");
    }

    #[test]
    fn prices_stay_money() {
        for text in [
            "It costs $5 and $10.",
            "Between $5-$10 per print",
            "$20,000 and $30,000",
            "Pay $5.",
            "A lone $ sign",
            "$ 5 $",
            "Escaped \\$x\\$ dollars",
        ] {
            assert_eq!(render_inline(text), text, "{text}");
            assert!(matches!(render_inline(text), Cow::Borrowed(_)), "{text}");
        }
    }

    #[test]
    fn math_after_money_still_converts() {
        assert_eq!(
            render_inline(r"From $5-$10, or $x \times 2$"),
            "From $5-$10, or x × 2"
        );
    }

    #[test]
    fn unknown_commands_stay_as_written() {
        assert_eq!(render_inline(r"$\foo$"), r"\foo");
        assert_eq!(render_inline(r"$\hat{x} + 1$"), r"\hat{x} + 1");
        assert_eq!(render_inline(r"$\binom{n}{k}$"), r"\binom{n}{k}");
    }

    #[test]
    fn code_spans_are_left_alone() {
        assert_eq!(
            render_inline(r"Run `echo $HOME $PATH` then $\to$ done"),
            r"Run `echo $HOME $PATH` then → done"
        );
        assert_eq!(render_inline(r"`$\rightarrow$`"), r"`$\rightarrow$`");
        assert_eq!(render_inline(r"$a `b` c$"), r"$a `b` c$");
        assert_eq!(
            render_inline(r"``a ` $\to$`` and $\to$"),
            r"``a ` $\to$`` and →"
        );
    }

    #[test]
    fn other_delimiters() {
        assert_eq!(render_inline(r"\(\alpha\) and \[x^2\]"), "α and x²");
        assert_eq!(render_inline(r"$$ E = mc^2 $$"), "E = mc²");
        assert_eq!(render_inline(r"\(  \)"), r"\(  \)");
    }

    /// The words of a one-paragraph document, as the parser sets them.
    fn paragraph(markdown: &str) -> String {
        let items: Vec<_> = super::super::parse(markdown).collect();
        let [super::super::Item::Paragraph(text)] = items.as_slice() else {
            panic!("one paragraph: {items:?}");
        };

        text.spans
            .iter()
            .map(|span| match span {
                super::super::Span::Standard { text, .. } => text.as_str(),
                #[cfg(feature = "highlighter")]
                super::super::Span::Highlight { text, .. } => text.as_str(),
            })
            .collect()
    }

    #[test]
    fn the_parser_sets_math() {
        assert_eq!(
            paragraph(r"*Order Received $\rightarrow$ Shipped*"),
            "Order Received → Shipped"
        );
        assert_eq!(paragraph(r"$$x^2$$"), "x²");
        assert_eq!(paragraph(r"`$\to$` and $\to$"), r"$\to$ and →");
        assert_eq!(paragraph("It costs $5 and $10."), "It costs $5 and $10.");
        assert_eq!(paragraph("$5-$10 a print"), "$5-$10 a print");
        assert_eq!(paragraph(r"Escaped \$x\$"), "Escaped $x$");
    }

    #[test]
    fn text_without_math_is_borrowed() {
        assert!(matches!(
            render_inline("Plain words, no math."),
            Cow::Borrowed(_)
        ));
    }
}
