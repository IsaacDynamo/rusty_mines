use std::collections::HashMap;
use anyhow::{anyhow, Result};
use pyo3::{prelude::*, types::PyDict};
use owo_colors::OwoColorize;
use clap::{Parser, Subcommand};

const SOURCE: &str = include_str!("../lib/decode_demcon3/mineField.py");

#[derive(Subcommand)]
enum Mode {
    Beginner,
    Intermediate,
    Expert,
}

#[derive(Debug)]
struct Minefield<'a> {
    field: &'a PyAny,
    width: i32,
    height: i32,
    number_of_mines: i32,
}

impl<'a> Minefield<'a> {
    fn new(py: Python<'a>, mode: Mode) -> Result<Self> {
        let module = PyModule::from_code(py, SOURCE, "mineField", "mineField")?;
        let minefield = module.getattr("MineField")?;

        let preset = match mode {
            Mode::Beginner => "BEGINNER_FIELD",
            Mode::Intermediate => "INTERMEDIATE_FIELD",
            Mode::Expert => "EXPERT_FIELD",
        };

        let kwargs = module
            .getattr(preset)?
            .downcast::<PyDict>()
            .map_err(|e| anyhow!("{}", e))?;

        let width: i32 = PyAny::get_item(kwargs, "width")?.extract()?;
        let height: i32 = PyAny::get_item(kwargs, "height")?.extract()?;
        let number_of_mines: i32 = PyAny::get_item(kwargs, "number_of_mines")?.extract()?;

        let field = minefield.call((), Some(kwargs))?;

        Ok(Self {
            field,
            width,
            height,
            number_of_mines,
        })
    }

    fn sweep_cell(&self, column: i32, row: i32) -> Result<u8> {
        let result = self.field.call_method("sweep_cell", (column, row), None)?;
        Ok(result.extract()?)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct Pos (i32, i32);

#[derive(Clone, Copy)]
enum Cell {
    Unknown,
    Flag,
    Number(u8),
}

struct Solver<'a> {
    minefield: &'a Minefield<'a>,
    board: Vec<Cell>,
}

impl<'a> Solver<'a> {
    fn new(minefield: &'a Minefield) -> Result<Self> {
        let size: usize = (minefield.width * minefield.height).try_into()?;
        Ok(Self {
            minefield,
            board: vec![Cell::Unknown; size],
        })
    }

    fn index(&self, pos: Pos) -> Option<usize> {
        let Pos(col, row) = pos;
        if col < 0 || col >= self.minefield.width || row < 0 || row >= self.minefield.height {
            return None;
        }

        let index: usize = (col + row * self.minefield.width).try_into().unwrap();
        Some(index)
    }

    fn get(&self, pos: Pos) -> Option<Cell> {
        self.index(pos).map(|i| self.board[i])
    }

    fn uncover(&mut self, pos: Pos) -> Result<Cell> {
        let Pos(col, row) = pos;
        let n = self.minefield.sweep_cell(col, row)?;
        let cell = Cell::Number(n);
        let i = self.index(pos).ok_or(anyhow!("Bad index"))?;
        self.board[i] = cell;
        Ok(cell)
    }

    fn plant_flag(&mut self, pos: Pos) -> Result<()> {
        let i = self.index(pos).ok_or(anyhow!("Bad index"))?;
        self.board[i] = Cell::Flag;
        Ok(())
    }

    fn neighbors(&self, pos: Pos) -> Vec<(Pos, Cell)> {
        let Pos(col, row) = pos;
        let list = [
            (1, 1),
            (1, 0),
            (1, -1),
            (0, 1),
            (0, -1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
        ];
        let r: Vec<(Pos, Cell)> = list
            .iter()
            .map(|(c, r)| {
                self.get(Pos(col + c, row + r))
                    .map(|cell| (Pos(col + c, row + r), cell))
            })
            .filter_map(|id| id)
            .collect();

        r
    }

    fn solve(&mut self) -> Result<()> {
        let mut active: Vec<Pos> = Vec::new();
        let mut next: Vec<Pos> = Vec::new();

        // First guess
        next.push(Pos(0, 0));

        loop {

            active.clear();
            std::mem::swap(&mut active, &mut next);
            let mut new_info = false;


            for pos in active.iter().copied() {
                let cell = self.get(pos).ok_or(anyhow!("Bad active cell location"))?;

                match cell {
                    Cell::Number(bombs) => {
                        let bombs: i32 = bombs.into();
                        let neighbors = self.neighbors(pos);
                        let flags: i32 = neighbors.iter().filter(|(_, cell)| matches!(cell, Cell::Flag)).count().try_into().unwrap();
                        let unknowns: i32 = neighbors.iter().filter(|(_, cell)| matches!(cell, Cell::Unknown)).count().try_into().unwrap();

                        if bombs == flags {
                            for p in neighbors.iter().filter_map(|(pos, cell)| matches!(cell, Cell::Unknown).then(|| *pos)) {
                                self.uncover(p)?;
                                next.push(p);
                            }
                            new_info = true;
                        } else if unknowns + flags == bombs {
                            for p in neighbors.iter().filter_map(|(pos, cell)| matches!(cell, Cell::Unknown).then(|| *pos)) {
                                self.plant_flag(p)?;
                            }
                            new_info = true;
                        } else {
                            next.push(pos);
                        }

                    }
                    Cell::Unknown => {
                        self.uncover(pos)?;
                        next.push(pos);
                        new_info = true;
                    }
                    _ => (),
                }
            }


            if new_info {
                continue;
            }

            // Simple algo didn't find new info, try heavier iterative algo now.

            let flags: i32 = self.board.iter().filter(|cell| matches!(cell, Cell::Flag)).count().try_into().unwrap();

            let mut unknowns = Vec::new();
            for col in 0..self.minefield.width {
                for row in 0..self.minefield.height {
                    let pos = Pos(col, row);
                    match self.get(pos) {
                        Some(Cell::Unknown) => unknowns.push(pos),
                        _ => (),
                    }
                }
            }

            // Already done
            if unknowns.len() == 0 {
                break;
            }

            let remaining_mines = self.minefield.number_of_mines - flags;
            let naive_chance = remaining_mines as f32 / unknowns.len() as f32;
            let mut probs: HashMap<Pos, f32> = unknowns.iter().copied().map(|pos| (pos, naive_chance)).collect();

            for i in 0..100 {

                let mut max_correction_diff = 0f32;

                for pos in active.iter().copied() {
                    let cell = self.get(pos).ok_or(anyhow!("Bad active cell location"))?;

                    match cell {
                        Cell::Number(bombs) => {
                            let bombs: i32 = bombs.into();
                            let neighbors = self.neighbors(pos);
                            let flags: i32 = neighbors.iter().filter(|(_, cell)| matches!(cell, Cell::Flag)).count().try_into().unwrap();
                            let unknowns: Vec<Pos> = neighbors.iter().filter_map(|(pos, cell)| matches!(cell, Cell::Unknown).then(|| (*pos))).collect();

                            let expected = (bombs - flags) as f32;
                            let sum: f32 = unknowns.iter().map(|pos| *probs.get(pos).unwrap()).sum();

                            let correction = expected / sum;

                            max_correction_diff = f32::max(max_correction_diff, f32::abs(1f32 - correction));

                            for pos in unknowns {
                                probs.get_mut(&pos).map(|p| *p *= correction);
                            }

                        }
                        _ => (),
                    }
                }

                // Normalize total prob
                let sum: f32 = probs.iter().map(|(_,p)| *p).sum();
                let correction = remaining_mines as f32 / sum;
                for (_, p) in probs.iter_mut() {
                    *p *= correction;
                }
                //dbg!(correction);

                max_correction_diff = f32::max(max_correction_diff, f32::abs(1f32 - correction));

                //dbg!(max_correction_diff);

                if max_correction_diff < 0.0001 {
                    dbg!(i);
                    break;
                }

            }

            let best_guess = probs.iter().min_by(|(_,p1), (_, p2)| (*p1).partial_cmp(*p2).unwrap()).unwrap();

            println!("{:?} {}", best_guess, naive_chance);

            let pos = *best_guess.0;
            let poke = self.uncover(pos);
            next.push(pos);

            if poke.is_err() {
                self.show();
            }
            poke?;

        }

        Ok(())
    }

    fn solved(&self) -> bool {
        let flags: i32 = self.board.iter().filter(|cell| matches!(cell, Cell::Flag)).count().try_into().unwrap();
        let unknowns: i32 = self.board.iter().filter(|cell| matches!(cell, Cell::Unknown)).count().try_into().unwrap();
        unknowns == 0 && flags == self.minefield.number_of_mines
    }

    fn show(&self) {
        for row in 0..self.minefield.height {
            for col in 0..self.minefield.width {
                match self.get(Pos(col, row)) {
                    Some(Cell::Flag) => print!("{} ", "F".bold().red()),
                    Some(Cell::Unknown) => print!(". "),
                    Some(Cell::Number(0)) => print!("  "),
                    Some(Cell::Number(x)) => print!("{} ", x),
                    _ => (),
                }
            }
            println!("");
        }
    }
}


#[derive(Parser)]
#[clap(about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    mode: Mode,
}

fn main() -> Result<()> {

    let cli = Cli::parse();

    Python::with_gil(|py| {
        let minefield = Minefield::new(py, cli.mode)?;

        let mut solver = Solver::new(&minefield)?;

        solver.solve()?;

        solver.show();

        println!("");
        println!("Solved: {}", solver.solved());

        Ok(())
    })
}
