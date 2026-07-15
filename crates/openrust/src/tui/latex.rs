//! LaTeX math notation → Unicode text conversion for terminal display.
//!
//! Handles Greek letters, superscripts/subscripts, fractions, roots,
//! mathematical operators, relations, arrows, set notation, font commands
//! (mathbb), and accents. Unknown commands fall through as plain text.

/// Convert LaTeX math notation to Unicode text.
pub fn latex_to_unicode(input: &str) -> String {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    p.parse(false)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn parse(&mut self, in_group: bool) -> String {
        let mut result = String::new();
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                '\\' => {
                    self.pos += 1;
                    self.handle_command(&mut result);
                }
                '^' => {
                    self.pos += 1;
                    let arg = self.read_arg();
                    push_super(&mut result, &arg);
                }
                '_' => {
                    self.pos += 1;
                    let arg = self.read_arg();
                    push_sub(&mut result, &arg);
                }
                '{' => {
                    self.pos += 1;
                    result.push_str(&self.parse(true));
                }
                '}' => {
                    self.pos += 1;
                    if in_group {
                        break;
                    }
                }
                '$' => {
                    self.pos += 1;
                }
                c => {
                    result.push(c);
                    self.pos += 1;
                }
            }
        }
        result
    }

    fn handle_command(&mut self, result: &mut String) {
        let cmd = self.read_command_name();
        match cmd.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.read_group();
                let den = self.read_group();
                // Parenthesise multi-char numerator / denominator to prevent
                // ambiguity: \frac{a+b}{c+d} → (a+b)/(c+d) not a+b/c+d.
                let needs_paren = |s: &str| s.len() > 1 && s.contains(['+', '-', '*', ' ']);
                let num_needs_paren = needs_paren(&num);
                let den_needs_paren = needs_paren(&den);
                if num_needs_paren {
                    result.push('(');
                }
                result.push_str(&num);
                if num_needs_paren {
                    result.push(')');
                }
                result.push('/');
                if den_needs_paren {
                    result.push('(');
                }
                result.push_str(&den);
                if den_needs_paren {
                    result.push(')');
                }
            }
            "sqrt" => {
                if let Some(n) = self.read_optional_bracket() {
                    result.push_str(&n);
                }
                result.push('√');
                let arg = self.read_group();
                if arg.chars().count() > 1 {
                    result.push('(');
                    result.push_str(&arg);
                    result.push(')');
                } else {
                    result.push_str(&arg);
                }
            }
            "binom" | "tbinom" | "dbinom" => {
                let n = self.read_group();
                let k = self.read_group();
                result.push_str("C(");
                result.push_str(&n);
                result.push(',');
                result.push_str(&k);
                result.push(')');
            }
            "text" | "textrm" | "textit" | "textbf" | "operatorname" | "mathrm" => {
                result.push_str(&self.read_group());
            }
            "mathbb" => result.push_str(&to_mathbb(&self.read_group())),
            "mathcal" | "mathbf" | "mathsf" | "mathtt" | "mathnormal" | "mathscr"
            | "boldsymbol" | "pmb" => {
                result.push_str(&self.read_group());
            }
            "hat" | "widehat" => accented(self.read_group(), '\u{0302}', result),
            "bar" | "overline" | "underline" => {
                accented(self.read_group(), '\u{0304}', result)
            }
            "vec" | "overrightarrow" => accented(self.read_group(), '\u{20D7}', result),
            "dot" => accented(self.read_group(), '\u{0307}', result),
            "ddot" => accented(self.read_group(), '\u{0308}', result),
            "tilde" | "widetilde" => accented(self.read_group(), '\u{0303}', result),
            "check" | "widecheck" => accented(self.read_group(), '\u{030C}', result),
            "breve" => accented(self.read_group(), '\u{0306}', result),
            "acute" => accented(self.read_group(), '\u{0301}', result),
            "grave" => accented(self.read_group(), '\u{0300}', result),
            "left" | "right" | "displaystyle" | "textstyle" | "scriptstyle"
            | "scriptscriptstyle" | "limits" | "nolimits" | "rm" | "bf" | "it" | "sf"
            | "tt" | "noalign" | "hline" | "centering" | "enspace" | "quad" | "qquad" => {}
            "," | ";" | ":" | "!" | "thinspace" | "medspace" | "thickspace" => {
                result.push(' ');
            }
            _ => {
                if let Some(sym) = lookup_symbol(&cmd) {
                    result.push_str(sym);
                } else {
                    result.push_str(&cmd);
                }
            }
        }
    }

    fn read_command_name(&mut self) -> String {
        if self.pos >= self.chars.len() {
            return String::new();
        }
        let c = self.chars[self.pos];
        if c.is_ascii_alphabetic() {
            let start = self.pos;
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_alphabetic() {
                self.pos += 1;
            }
            self.chars[start..self.pos].iter().collect()
        } else {
            self.pos += 1;
            c.to_string()
        }
    }

    fn read_arg(&mut self) -> String {
        if self.pos >= self.chars.len() {
            return String::new();
        }
        match self.chars[self.pos] {
            '{' => self.read_group(),
            '\\' => {
                self.pos += 1;
                let cmd = self.read_command_name();
                lookup_symbol(&cmd).unwrap_or(&cmd).to_string()
            }
            c => {
                self.pos += 1;
                c.to_string()
            }
        }
    }

    fn read_group(&mut self) -> String {
        if self.pos >= self.chars.len() {
            return String::new();
        }
        if self.chars[self.pos] == '{' {
            self.pos += 1;
            self.parse(true)
        } else {
            self.read_arg()
        }
    }

    fn read_optional_bracket(&mut self) -> Option<String> {
        if self.pos < self.chars.len() && self.chars[self.pos] == '[' {
            self.pos += 1;
            let mut depth = 1;
            let mut result = String::new();
            while self.pos < self.chars.len() && depth > 0 {
                match self.chars[self.pos] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            self.pos += 1;
                            return Some(result);
                        }
                    }
                    _ => {}
                }
                if depth > 0 {
                    result.push(self.chars[self.pos]);
                }
                self.pos += 1;
            }
            Some(result)
        } else {
            None
        }
    }
}

fn accented(text: String, combining: char, result: &mut String) {
    result.push_str(&text);
    result.push(combining);
}

fn push_super(result: &mut String, arg: &str) {
    if let Some(s) = to_superscript(arg) {
        result.push_str(&s);
    } else {
        result.push_str("^(");
        result.push_str(arg);
        result.push(')');
    }
}

fn push_sub(result: &mut String, arg: &str) {
    if let Some(s) = to_subscript(arg) {
        result.push_str(&s);
    } else {
        result.push_str("_(");
        result.push_str(arg);
        result.push(')');
    }
}

pub(super) fn to_superscript(s: &str) -> Option<String> {
    let mut result = String::new();
    for c in s.chars() {
        result.push(super_char(c)?);
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub(super) fn to_subscript(s: &str) -> Option<String> {
    let mut result = String::new();
    for c in s.chars() {
        result.push(sub_char(c)?);
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn super_char(c: char) -> Option<char> {
    match c {
        '0' => Some('⁰'), '1' => Some('¹'), '2' => Some('²'), '3' => Some('³'),
        '4' => Some('⁴'), '5' => Some('⁵'), '6' => Some('⁶'), '7' => Some('⁷'),
        '8' => Some('⁸'), '9' => Some('⁹'), '+' => Some('⁺'), '-' => Some('⁻'),
        '=' => Some('⁼'), '(' => Some('⁽'), ')' => Some('⁾'),
        'a' => Some('ᵃ'), 'b' => Some('ᵇ'), 'c' => Some('ᶜ'), 'd' => Some('ᵈ'),
        'e' => Some('ᵉ'), 'f' => Some('ᶠ'), 'g' => Some('ᵍ'), 'h' => Some('ʰ'),
        'i' => Some('ⁱ'), 'j' => Some('ʲ'), 'k' => Some('ᵏ'), 'l' => Some('ˡ'),
        'm' => Some('ᵐ'), 'n' => Some('ⁿ'), 'o' => Some('ᵒ'), 'p' => Some('ᵖ'),
        'r' => Some('ʳ'), 's' => Some('ˢ'), 't' => Some('ᵗ'), 'u' => Some('ᵘ'),
        'v' => Some('ᵛ'), 'w' => Some('ʷ'), 'x' => Some('ˣ'), 'y' => Some('ʸ'),
        'z' => Some('ᶻ'),
        'A' => Some('ᴬ'), 'B' => Some('ᴮ'), 'D' => Some('ᴰ'), 'E' => Some('ᴱ'),
        'G' => Some('ᴳ'), 'H' => Some('ᴴ'), 'I' => Some('ᴵ'), 'J' => Some('ᴶ'),
        'K' => Some('ᴷ'), 'L' => Some('ᴸ'), 'M' => Some('ᴹ'), 'N' => Some('ᴺ'),
        'O' => Some('ᴼ'), 'P' => Some('ᴾ'), 'R' => Some('ᴿ'), 'T' => Some('ᵀ'),
        'U' => Some('ᵁ'), 'V' => Some('ⱽ'), 'W' => Some('ᵂ'),
        ',' => Some('︐'), '.' => Some('˙'),
        _ => None,
    }
}

fn sub_char(c: char) -> Option<char> {
    match c {
        '0' => Some('₀'), '1' => Some('₁'), '2' => Some('₂'), '3' => Some('₃'),
        '4' => Some('₄'), '5' => Some('₅'), '6' => Some('₆'), '7' => Some('₇'),
        '8' => Some('₈'), '9' => Some('₉'), '+' => Some('₊'), '-' => Some('₋'),
        '=' => Some('₌'), '(' => Some('₍'), ')' => Some('₎'),
        'a' => Some('ₐ'), 'e' => Some('ₑ'), 'h' => Some('ₕ'), 'i' => Some('ᵢ'),
        'j' => Some('ⱼ'), 'k' => Some('ₖ'), 'l' => Some('ₗ'), 'm' => Some('ₘ'),
        'n' => Some('ₙ'), 'o' => Some('ₒ'), 'p' => Some('ₚ'), 'r' => Some('ᵣ'),
        's' => Some('ₛ'), 't' => Some('ₜ'), 'u' => Some('ᵤ'), 'v' => Some('ᵥ'),
        'x' => Some('ₓ'),
        _ => None,
    }
}

fn to_mathbb(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A' => '\u{1D538}', 'B' => '\u{1D539}', 'C' => '\u{2102}',
            'D' => '\u{1D53B}', 'E' => '\u{1D53C}', 'F' => '\u{1D53D}',
            'G' => '\u{1D53E}', 'H' => '\u{210D}', 'I' => '\u{1D540}',
            'J' => '\u{1D541}', 'K' => '\u{1D542}', 'L' => '\u{1D543}',
            'M' => '\u{1D544}', 'N' => '\u{2115}', 'O' => '\u{1D546}',
            'P' => '\u{2119}', 'Q' => '\u{211A}', 'R' => '\u{211D}',
            'S' => '\u{1D54A}', 'T' => '\u{1D54B}', 'U' => '\u{1D54C}',
            'V' => '\u{1D54D}', 'W' => '\u{1D54E}', 'X' => '\u{1D54F}',
            'Y' => '\u{1D550}', 'Z' => '\u{2124}',
            _ => c,
        })
        .collect()
}

fn lookup_symbol(cmd: &str) -> Option<&'static str> {
    let sym = match cmd {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ",
        "epsilon" => "ε", "varepsilon" => "ε", "zeta" => "ζ", "eta" => "η",
        "theta" => "θ", "vartheta" => "ϑ", "iota" => "ι", "kappa" => "κ",
        "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ",
        "omicron" => "ο", "pi" => "π", "varpi" => "ϖ", "rho" => "ρ",
        "varrho" => "ϱ", "sigma" => "σ", "varsigma" => "ς", "tau" => "τ",
        "upsilon" => "υ", "phi" => "φ", "varphi" => "ϕ", "chi" => "χ",
        "psi" => "ψ", "omega" => "ω",
        "Alpha" => "Α", "Beta" => "Β", "Gamma" => "Γ", "Delta" => "Δ",
        "Epsilon" => "Ε", "Zeta" => "Ζ", "Eta" => "Η", "Theta" => "Θ",
        "Iota" => "Ι", "Kappa" => "Κ", "Lambda" => "Λ", "Mu" => "Μ",
        "Nu" => "Ν", "Xi" => "Ξ", "Omicron" => "Ο", "Pi" => "Π",
        "Rho" => "Ρ", "Sigma" => "Σ", "Tau" => "Τ", "Upsilon" => "Υ",
        "Phi" => "Φ", "Chi" => "Χ", "Psi" => "Ψ", "Omega" => "Ω",
        "sum" => "Σ", "prod" => "∏", "coprod" => "∐", "int" => "∫",
        "oint" => "∮", "iint" => "∬", "iiint" => "∭", "bigcap" => "⋂",
        "bigcup" => "⋃", "bigvee" => "⋁", "bigwedge" => "⋀",
        "bigoplus" => "⨁", "bigotimes" => "⨂", "bigodot" => "⨀",
        "biguplus" => "⨄",
        "times" => "×", "div" => "÷", "cdot" => "·", "pm" => "±",
        "mp" => "∓", "ast" => "∗", "star" => "⋆", "circ" => "∘",
        "bullet" => "∙", "cap" => "∩", "cup" => "∪", "vee" => "∨",
        "wedge" => "∧", "oplus" => "⊕", "ominus" => "⊖", "otimes" => "⊗",
        "odot" => "⊙", "oslash" => "⊘", "setminus" => "∖", "wr" => "≀",
        "dagger" => "†", "ddagger" => "‡",
        "leq" => "≤", "le" => "≤", "geq" => "≥", "ge" => "≥",
        "neq" => "≠", "ne" => "≠", "approx" => "≈", "equiv" => "≡",
        "sim" => "∼", "simeq" => "≃", "cong" => "≅", "propto" => "∝",
        "prec" => "≺", "succ" => "≻", "preceq" => "⪯", "succeq" => "⪰",
        "subset" => "⊂", "supset" => "⊃", "subseteq" => "⊆", "supseteq" => "⊇",
        "sqsubset" => "⊏", "sqsupset" => "⊐", "sqsubseteq" => "⊑",
        "sqsupseteq" => "⊒", "in" => "∈", "notin" => "∉", "ni" => "∋",
        "vdash" => "⊢", "dashv" => "⊣", "models" => "⊧", "perp" => "⊥",
        "parallel" => "∥", "mid" => "∣", "nmid" => "∤", "ll" => "≪",
        "gg" => "≫",
        "rightarrow" => "→", "to" => "→", "leftarrow" => "←", "gets" => "←",
        "Rightarrow" => "⇒", "Leftarrow" => "⇐", "Leftrightarrow" => "⇔",
        "iff" => "⇔", "leftrightarrow" => "↔", "mapsto" => "↦",
        "hookrightarrow" => "↪", "hookleftarrow" => "↩", "uparrow" => "↑",
        "downarrow" => "↓", "Uparrow" => "⇑", "Downarrow" => "⇓",
        "updownarrow" => "↕", "Updownarrow" => "⇕", "nearrow" => "↗",
        "searrow" => "↘", "nwarrow" => "↖", "swarrow" => "↙",
        "rightharpoonup" => "⇀", "rightharpoondown" => "⇁",
        "leftharpoonup" => "↼", "leftharpoondown" => "↽",
        "rightleftharpoons" => "⇌", "overleftrightarrow" => "↔",
        "infty" => "∞", "partial" => "∂", "nabla" => "∇",
        "forall" => "∀", "exists" => "∃", "nexists" => "∄",
        "neg" => "¬", "lnot" => "¬", "emptyset" => "∅", "varnothing" => "∅",
        "aleph" => "ℵ", "beth" => "ℶ", "ell" => "ℓ", "hbar" => "ℏ",
        "imath" => "ı", "jmath" => "ȷ", "Re" => "ℜ", "Im" => "ℑ",
        "wp" => "℘", "complement" => "∁", "angle" => "∠",
        "measuredangle" => "∡", "prime" => "′", "flat" => "♭",
        "natural" => "♮", "sharp" => "♯", "square" => "□",
        "blacksquare" => "■", "triangle" => "△", "blacktriangle" => "▲",
        "diamond" => "◇", "lozenge" => "◊", "bigcirc" => "◯",
        "cdots" => "⋯", "vdots" => "⋮", "ddots" => "⋱", "ldots" => "…",
        "dots" => "…", "colon" => ":",
        "{" => "{", "}" => "}", "%" => "%", "#" => "#", "&" => "&",
        "_" => "_", "$" => "$", "textbackslash" => "\\",
        "degree" => "°", "pounds" => "£", "euros" => "€", "yen" => "¥",
        "checkmark" => "✓", "heartsuit" => "♥", "spadesuit" => "♠",
        "diamondsuit" => "♦", "clubsuit" => "♣",
        _ => return None,
    };
    Some(sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greek_letters() {
        assert_eq!(latex_to_unicode(r"\alpha + \beta"), "α + β");
        assert_eq!(latex_to_unicode(r"\Gamma \Delta"), "Γ Δ");
    }

    #[test]
    fn superscripts() {
        assert_eq!(latex_to_unicode(r"x^2"), "x²");
        assert_eq!(latex_to_unicode(r"2^{10}"), "2¹⁰");
        assert_eq!(latex_to_unicode(r"e^{i\pi}"), "e^(iπ)");
    }

    #[test]
    fn subscripts() {
        assert_eq!(latex_to_unicode(r"x_n"), "xₙ");
        assert_eq!(latex_to_unicode(r"a_{ij}"), "aᵢⱼ");
    }

    #[test]
    fn fractions() {
        assert_eq!(latex_to_unicode(r"\frac{a}{b}"), "a/b");
        assert_eq!(latex_to_unicode(r"\frac{1}{2}"), "1/2");
        assert_eq!(latex_to_unicode(r"\frac{a+b}{c+d}"), "(a+b)/(c+d)");
        assert_eq!(latex_to_unicode(r"\frac{x}{y+1}"), "x/(y+1)");
    }

    #[test]
    fn sqrt() {
        assert_eq!(latex_to_unicode(r"\sqrt{x}"), "√x");
        assert_eq!(latex_to_unicode(r"\sqrt{x+1}"), "√(x+1)");
    }

    #[test]
    fn operators() {
        assert_eq!(latex_to_unicode(r"\sum_{i=0}^{n}"), "Σᵢ₌₀ⁿ");
        assert_eq!(latex_to_unicode(r"\int_0^1"), "∫₀¹");
        assert_eq!(latex_to_unicode(r"a \times b"), "a × b");
    }

    #[test]
    fn relations() {
        assert_eq!(latex_to_unicode(r"a \leq b"), "a ≤ b");
        assert_eq!(latex_to_unicode(r"x \neq y"), "x ≠ y");
        assert_eq!(latex_to_unicode(r"a \approx b"), "a ≈ b");
    }

    #[test]
    fn arrows() {
        assert_eq!(latex_to_unicode(r"x \to y"), "x → y");
        assert_eq!(latex_to_unicode(r"a \Rightarrow b"), "a ⇒ b");
        assert_eq!(latex_to_unicode(r"f \mapsto g"), "f ↦ g");
    }

    #[test]
    fn mathbb() {
        assert_eq!(latex_to_unicode(r"\mathbb{R}"), "ℝ");
        assert_eq!(latex_to_unicode(r"\mathbb{N}"), "ℕ");
        assert_eq!(latex_to_unicode(r"\mathbb{Z}"), "ℤ");
        assert_eq!(latex_to_unicode(r"\mathbb{E}"), "𝔼");
        assert_eq!(latex_to_unicode(r"\mathbb{A}"), "𝔸");
    }

    #[test]
    fn einstein() {
        assert_eq!(latex_to_unicode(r"E = mc^2"), "E = mc²");
    }

    #[test]
    fn dollar_delimiters() {
        assert_eq!(latex_to_unicode("$E = mc^2$"), "E = mc²");
        assert_eq!(latex_to_unicode("$$\\sum_{i=1}^n x_i$$"), "Σᵢ₌₁ⁿ xᵢ");
    }

    #[test]
    fn unknown_command_falls_through() {
        assert_eq!(latex_to_unicode(r"\foobar"), "foobar");
    }

    #[test]
    fn escaped_special() {
        assert_eq!(latex_to_unicode(r"\{ \} \% \# \&"), "{ } % # &");
    }

    #[test]
    fn nested_groups() {
        assert_eq!(
            latex_to_unicode(r"\frac{\sqrt{x}}{2}"),
            "√x/2"
        );
    }

    #[test]
    fn binom() {
        assert_eq!(latex_to_unicode(r"\binom{n}{k}"), "C(n,k)");
    }
}
