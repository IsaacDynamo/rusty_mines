use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use pyo3::{prelude::*, types::PyDict};
use std::collections::HashMap;

const SOURCE: &str = include_str!("../lib/decode_demcon3/mineField.py");

#[derive(Subcommand, Copy, Clone, Debug)]
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

    fn sweep_cell(&self, column: i32, row: i32) -> Result<Cell> {
        let result = self.field.call_method("sweep_cell", (column, row), None);
        match result {
            Ok(result) => Ok(Cell::Number(result.extract()?)),
            Err(e) if format!("{}", e) == "ExplosionException: " => Ok(Cell::Mine),
            Err(e) => Err(e.into()),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct Pos(i32, i32);

#[derive(Clone, Copy)]
enum Cell {
    Unknown,
    Flag,
    Number(u8),
    Mine,
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
        let cell = self.minefield.sweep_cell(col, row)?;
        let i = self.index(pos).ok_or_else(|| anyhow!("Bad index"))?;
        self.board[i] = cell;
        Ok(cell)
    }

    fn plant_flag(&mut self, pos: Pos) -> Result<()> {
        let i = self.index(pos).ok_or_else(|| anyhow!("Bad index"))?;
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
            .filter_map(|(c, r)| {
                self.get(Pos(col + c, row + r))
                    .map(|cell| (Pos(col + c, row + r), cell))
            })
            .collect();

        r
    }

    fn solve(&mut self) -> Result<bool> {
        let mut active: Vec<Pos> = Vec::new();

        // First guess: 0,0 why not
        let mut next = vec![Pos(0, 0)];

        loop {
            active.clear();
            std::mem::swap(&mut active, &mut next);
            let mut new_info = false;

            for pos in active.iter().copied() {
                let cell = self
                    .get(pos)
                    .ok_or_else(|| anyhow!("Bad active cell location"))?;

                match cell {
                    Cell::Number(mines) => {
                        let mines: i32 = mines.into();
                        let neighbors = self.neighbors(pos);
                        let flags: i32 = neighbors
                            .iter()
                            .filter(|(_, cell)| matches!(cell, Cell::Flag))
                            .count()
                            .try_into()
                            .unwrap();
                        let unknowns: i32 = neighbors
                            .iter()
                            .filter(|(_, cell)| matches!(cell, Cell::Unknown))
                            .count()
                            .try_into()
                            .unwrap();

                        if unknowns == 0 {
                            // Done
                        } else if mines == flags {
                            for p in neighbors.iter().filter_map(|(pos, cell)| {
                                matches!(cell, Cell::Unknown).then(|| *pos)
                            }) {
                                self.uncover(p)?;
                                next.push(p);
                            }
                            new_info = true;
                        } else if unknowns + flags == mines {
                            for p in neighbors.iter().filter_map(|(pos, cell)| {
                                matches!(cell, Cell::Unknown).then(|| *pos)
                            }) {
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
                    Cell::Mine => return Ok(false),
                    _ => (),
                }
            }

            if new_info {
                continue;
            }

            // Simple algo didn't find new info, try heavier iterative algo now.

            let flags: i32 = self
                .board
                .iter()
                .filter(|cell| matches!(cell, Cell::Flag))
                .count()
                .try_into()
                .unwrap();

            let mut unknowns = Vec::new();
            for col in 0..self.minefield.width {
                for row in 0..self.minefield.height {
                    let pos = Pos(col, row);
                    if let Some(Cell::Unknown) = self.get(pos) {
                        unknowns.push(pos)
                    }
                }
            }

            // Already done
            if unknowns.is_empty() {
                break;
            }

            let remaining_mines = self.minefield.number_of_mines - flags;

            // Uncover remaining cells when all mines are flagged, then we are done
            if remaining_mines == 0 {
                for pos in unknowns {
                    self.uncover(pos)?;
                }
                break;
            }

            let naive_chance = remaining_mines as f32 / unknowns.len() as f32;
            let mut probs: HashMap<Pos, f32> = unknowns
                .iter()
                .copied()
                .map(|pos| (pos, naive_chance))
                .collect();

            for _ in 0..100 {
                let mut max_correction_diff = 0f32;

                for pos in active.iter().copied() {
                    let cell = self
                        .get(pos)
                        .ok_or_else(|| anyhow!("Bad active cell location"))?;

                    if let Cell::Number(mines) = cell {
                        let mines: i32 = mines.into();
                        let neighbors = self.neighbors(pos);
                        let flags: i32 = neighbors
                            .iter()
                            .filter(|(_, cell)| matches!(cell, Cell::Flag))
                            .count()
                            .try_into()
                            .unwrap();
                        let unknowns: Vec<Pos> = neighbors
                            .iter()
                            .filter_map(|(pos, cell)| matches!(cell, Cell::Unknown).then(|| (*pos)))
                            .collect();

                        let expected = (mines - flags) as f32;
                        let sum: f32 = unknowns.iter().map(|pos| *probs.get(pos).unwrap()).sum();
                        let correction = expected / sum;

                        max_correction_diff =
                            f32::max(max_correction_diff, f32::abs(1f32 - correction));

                        for pos in unknowns {
                            if let Some(p) = probs.get_mut(&pos) {
                                *p *= correction;
                            }
                        }
                    }
                }

                // Normalize total prob
                let sum: f32 = probs.iter().map(|(_, p)| p).copied().sum();
                let correction = remaining_mines as f32 / sum;
                for (_, p) in probs.iter_mut() {
                    *p *= correction;
                }

                max_correction_diff = f32::max(max_correction_diff, f32::abs(1f32 - correction));

                // Enough conversion, done iterating
                if max_correction_diff < 0.0001 {
                    break;
                }
            }

            let best_guess = probs
                .iter()
                .min_by(|(_, p1), (_, p2)| (*p1).partial_cmp(*p2).unwrap())
                .unwrap();

            //println!("{:?} {}", best_guess, naive_chance);

            let pos = *best_guess.0;
            let cell = self.uncover(pos)?;
            if let Cell::Mine = cell {
                return Ok(false);
            }
            next.push(pos);
        }

        Ok(self.solved())
    }

    fn solved(&self) -> bool {
        let flags: i32 = self
            .board
            .iter()
            .filter(|cell| matches!(cell, Cell::Flag))
            .count()
            .try_into()
            .unwrap();
        let unknowns: i32 = self
            .board
            .iter()
            .filter(|cell| matches!(cell, Cell::Unknown))
            .count()
            .try_into()
            .unwrap();
        let mines: i32 = self
            .board
            .iter()
            .filter(|cell| matches!(cell, Cell::Mine))
            .count()
            .try_into()
            .unwrap();
        unknowns == 0 && mines == 0 && flags == self.minefield.number_of_mines
    }

    fn show(&self) {
        for row in 0..self.minefield.height {
            for col in 0..self.minefield.width {
                match self.get(Pos(col, row)).unwrap() {
                    Cell::Flag => print!("{} ", "F".bold().cyan()),
                    Cell::Unknown => print!(". "),
                    Cell::Number(0) => print!("  "),
                    Cell::Number(x) => print!("{} ", x),
                    Cell::Mine => print!("{} ", "X".bold().red()),
                }
            }
            println!();
        }
    }
}

#[derive(Parser)]
#[clap(about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    mode: Mode,

    #[clap(short, long, value_parser)]
    iterations: Option<usize>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    Python::with_gil(|py| {
        if let Some(iterations) = cli.iterations {
            let mut success = 0;
            for _ in 0..iterations {
                let minefield = Minefield::new(py, cli.mode)?;
                let mut solver = Solver::new(&minefield)?;
                if solver.solve()? {
                    success += 1;
                }
            }

            println!(
                "Solved {}/{} successful, {:?}",
                success, iterations, cli.mode
            );
        } else {
            let minefield = Minefield::new(py, cli.mode)?;
            let mut solver = Solver::new(&minefield)?;

            let solved = solver.solve()?;
            solver.show();

            println!();
            println!("Solved: {}", solved);
        }

        Ok(())
    })
}
