//! A tiny column-layout engine: measure, shrink to terminal width, print.
//!
//! Cells arrive already coloured, so every measurement goes through
//! [`fmt::visible_width`].

use crate::fmt;
use crate::theme;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub struct Column {
    pub title: String,
    pub align: Align,
    /// Flexible columns give up width first when the table overflows.
    pub flex: bool,
    /// Never shrink below this many columns.
    pub min: usize,
    /// Never grow past this share of the terminal, as a percentage. One long
    /// outlier (a swarm task name, a digest) should not push every other
    /// column across the screen.
    pub cap_pct: Option<usize>,
}

impl Column {
    pub fn left(title: impl Into<String>) -> Self {
        Column { title: title.into(), align: Align::Left, flex: false, min: 0, cap_pct: None }
    }
    pub fn right(title: impl Into<String>) -> Self {
        Column { title: title.into(), align: Align::Right, flex: false, min: 0, cap_pct: None }
    }
    pub fn flex(mut self, min: usize) -> Self {
        self.flex = true;
        self.min = min;
        self
    }

    /// Cap this column at `pct` percent of the terminal width.
    pub fn cap(mut self, pct: usize) -> Self {
        self.cap_pct = Some(pct);
        self
    }
}

pub enum Row {
    Cells(Vec<String>),
    /// A section heading (compose project, network, …) spanning the table.
    Group(String),
    Blank,
}

pub struct Table {
    cols: Vec<Column>,
    rows: Vec<Row>,
    gutter: usize,
}

impl Table {
    pub fn new(cols: Vec<Column>) -> Self {
        Table { cols, rows: Vec::new(), gutter: theme::l().gutter }
    }

    pub fn row<I: IntoIterator<Item = String>>(&mut self, cells: I) {
        self.rows.push(Row::Cells(cells.into_iter().collect()));
    }

    pub fn group(&mut self, title: String) {
        self.rows.push(Row::Group(title));
    }

    pub fn blank(&mut self) {
        self.rows.push(Row::Blank);
    }

    /// Compute widths, shrink flexible columns to fit, then write to stdout.
    pub fn print(&self) {
        let n = self.cols.len();
        let mut widths: Vec<usize> =
            self.cols.iter().map(|c| fmt::visible_width(&c.title)).collect();
        for row in &self.rows {
            if let Row::Cells(cells) = row {
                for (i, cell) in cells.iter().take(n).enumerate() {
                    widths[i] = widths[i].max(fmt::visible_width(cell));
                }
            }
        }

        // Apply the per-column caps before fitting: a column is allowed to be
        // as wide as its content only up to its share of the screen.
        let term = fmt::term_width();
        for (i, col) in self.cols.iter().enumerate() {
            if let Some(pct) = col.cap_pct {
                let cap = (term * pct / 100).max(col.min).max(fmt::visible_width(&col.title));
                widths[i] = widths[i].min(cap);
            }
        }

        let sep_w = theme::l().column_sep.map(fmt::visible_width).unwrap_or(0);
        let gutters = (self.gutter + sep_w) * n.saturating_sub(1);
        let avail = term.saturating_sub(gutters);
        let mut total: usize = widths.iter().sum();

        // Shrink flexible columns, widest first, until the table fits.
        while total > avail {
            let victim = self
                .cols
                .iter()
                .enumerate()
                .filter(|(i, c)| c.flex && widths[*i] > c.min)
                .max_by_key(|(i, _)| widths[*i])
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    let cut = (total - avail).min(widths[i] - self.cols[i].min);
                    widths[i] -= cut.max(1);
                    total = widths.iter().sum();
                }
                None => break,
            }
        }

        // Column separator, padded by the gutter on both sides when present.
        let half = self.gutter.div_ceil(2);
        let joiner = match theme::l().column_sep {
            Some(sep) => {
                format!("{}{}{}", " ".repeat(half), theme::dim(sep), " ".repeat(self.gutter - half))
            }
            None => " ".repeat(self.gutter),
        };

        // Header
        let header: Vec<String> = self
            .cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let t = theme::header(&fmt::truncate(&c.title, widths[i]));
                match c.align {
                    Align::Left => fmt::pad(&t, widths[i]),
                    Align::Right => fmt::rpad(&t, widths[i]),
                }
            })
            .collect();
        println!("{}", header.join(&joiner));

        // Optional rule under the header, spanning the printed width.
        if let Some(rule) = theme::l().rule {
            let span = widths.iter().sum::<usize>() + gutters;
            let unit = fmt::visible_width(rule).max(1);
            println!("{}", theme::dim(&rule.repeat(span / unit)));
        }

        let indent = joiner;
        for row in &self.rows {
            match row {
                Row::Blank => println!(),
                Row::Group(title) => println!("{title}"),
                Row::Cells(cells) => {
                    let mut line = String::new();
                    for (i, width) in widths.iter().enumerate().take(n) {
                        if i > 0 {
                            line.push_str(&indent);
                        }
                        let raw = cells.get(i).map(String::as_str).unwrap_or("");
                        let t = fmt::truncate(raw, *width);
                        let cell = match self.cols[i].align {
                            Align::Left => fmt::pad(&t, *width),
                            Align::Right => fmt::rpad(&t, *width),
                        };
                        line.push_str(&cell);
                    }
                    println!("{}", line.trim_end());
                }
            }
        }
    }
}
