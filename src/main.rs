use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use pyo3::{prelude::*, types::PyDict};
use rand::{thread_rng, Rng};
use std::collections::HashMap;

const SOURCE: &str = include_str!("../lib/decode_demcon3/mineField.py");

const NEIGHBORS: [(i32, i32); 8] = [
    (1, 1),
    (1, 0),
    (1, -1),
    (0, 1),
    (0, -1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

#[derive(Subcommand, Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum Mode {
    Beginner,
    Intermediate,
    Expert,
}

struct MinefieldBuilder<'a> {
    class: &'a PyAny,
    presets: HashMap<Mode, (i32, i32, i32, &'a PyDict)>,
}

impl<'a> MinefieldBuilder<'a> {
    fn new(py: Python<'a>) -> Result<Self> {
        let module = PyModule::from_code(py, SOURCE, "mineField", "mineField")?;
        let class = module.getattr("MineField")?;

        let list = [
            (Mode::Beginner, "BEGINNER_FIELD"),
            (Mode::Intermediate, "INTERMEDIATE_FIELD"),
            (Mode::Expert, "EXPERT_FIELD"),
        ];

        let presets = list
            .iter()
            .map(|(mode, name)| {
                let kwargs = module
                    .getattr(name)?
                    .downcast::<PyDict>()
                    .map_err(|e| anyhow!("{}", e))?;

                let width: i32 = PyAny::get_item(kwargs, "width")?.extract()?;
                let height: i32 = PyAny::get_item(kwargs, "height")?.extract()?;
                let number_of_mines: i32 = PyAny::get_item(kwargs, "number_of_mines")?.extract()?;

                Ok((*mode, (width, height, number_of_mines, kwargs)))
            })
            .collect::<Result<HashMap<Mode, (i32, i32, i32, &PyDict)>>>()?;

        Ok(Self { class, presets })
    }

    fn build(&self, mode: Mode) -> Result<PythonMinefield<'a>> {
        let args = self
            .presets
            .get(&mode)
            .ok_or_else(|| anyhow!("Mode not found"))?;
        let field = self.class.call((), Some(args.3))?;

        Ok(PythonMinefield {
            field,
            width: args.0,
            height: args.1,
            number_of_mines: args.2,
        })
    }
}

trait Minefield {
    fn sweep_cell(&mut self, column: i32, row: i32) -> Result<Cell>;
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn number_of_mines(&self) -> i32;
}

#[derive(Debug)]
struct PythonMinefield<'a> {
    field: &'a PyAny,
    width: i32,
    height: i32,
    number_of_mines: i32,
}

impl<'a> Minefield for PythonMinefield<'a> {
    fn sweep_cell(&mut self, column: i32, row: i32) -> Result<Cell> {
        let result = self.field.call_method("sweep_cell", (column, row), None);
        match result {
            Ok(result) => Ok(Cell::Number(result.extract()?)),
            Err(e) if format!("{}", e) == "ExplosionException: " => Ok(Cell::Mine),
            Err(e) => Err(e.into()),
        }
    }

    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    fn number_of_mines(&self) -> i32 {
        self.number_of_mines
    }
}

struct RustMinefield {
    field: Vec<bool>,
    width: i32,
    height: i32,
    number_of_mines: i32,
}

impl RustMinefield {
    fn new(mode: Mode) -> Self {
        let (width, height, number_of_mines) = match mode {
            Mode::Beginner => (10, 10, 10),
            Mode::Intermediate => (16, 16, 40),
            Mode::Expert => (30, 16, 99),
        };

        Self {
            field: Vec::new(),
            width,
            height,
            number_of_mines,
        }
    }

    fn get(&mut self, col: i32, row: i32) -> Option<bool> {
        if col < 0 || col >= self.width || row < 0 || row >= self.height {
            return None;
        }

        let index: usize = (col + row * self.width).try_into().unwrap();

        if self.field.is_empty() {
            let size: usize = (self.width * self.height).try_into().unwrap();
            self.field = vec![false; size];

            let mut rng = thread_rng();

            let mut mines_left = self.number_of_mines;
            while mines_left != 0 {
                let random_index = rng.gen_range(0..size);
                if random_index != index && !self.field[random_index] {
                    self.field[random_index] = true;
                    mines_left -= 1;
                }
            }
        }

        Some(self.field[index])
    }

    fn neighbors(&mut self, col: i32, row: i32) -> u8 {
        NEIGHBORS
            .iter()
            .map(|(c, r)| -> u8 { self.get(col + c, row + r).unwrap_or(false).into() })
            .sum()
    }
}

impl Minefield for RustMinefield {
    fn sweep_cell(&mut self, column: i32, row: i32) -> Result<Cell> {
        match self.get(column, row).unwrap() {
            true => Ok(Cell::Mine),
            false => Ok(Cell::Number(self.neighbors(column, row))),
        }
    }

    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    fn number_of_mines(&self) -> i32 {
        self.number_of_mines
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct Pos(i32, i32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Unknown,
    Flag,
    Number(u8),
    Mine,
}

struct Solver<'a, T: Minefield> {
    minefield: &'a mut T,
    board: Vec<Cell>,
    flags: i32,
    unknowns: i32,
}

impl<'a, T: Minefield> Solver<'a, T> {
    fn new(minefield: &'a mut T) -> Result<Self> {
        let size: usize = (minefield.width() * minefield.height()).try_into()?;
        Ok(Self {
            minefield,
            board: vec![Cell::Unknown; size],
            flags: 0,
            unknowns: size.try_into().unwrap(),
        })
    }

    fn index(&self, pos: Pos) -> Option<usize> {
        let Pos(col, row) = pos;
        if col < 0 || col >= self.minefield.width() || row < 0 || row >= self.minefield.height() {
            return None;
        }

        let index: usize = (col + row * self.minefield.width()).try_into().unwrap();
        Some(index)
    }

    fn get(&self, pos: Pos) -> Option<Cell> {
        self.index(pos).map(|i| self.board[i])
    }

    fn uncover(&mut self, pos: Pos) -> Result<Cell> {
        let Pos(col, row) = pos;
        let cell = self.minefield.sweep_cell(col, row)?;
        let i = self.index(pos).ok_or_else(|| anyhow!("Bad index"))?;
        assert!(self.board[i] == Cell::Unknown);
        self.board[i] = cell;
        self.unknowns -= 1;
        Ok(cell)
    }

    fn plant_flag(&mut self, pos: Pos) -> Result<()> {
        let i = self.index(pos).ok_or_else(|| anyhow!("Bad index"))?;
        assert!(self.board[i] == Cell::Unknown);
        self.board[i] = Cell::Flag;
        self.flags += 1;
        self.unknowns -= 1;
        Ok(())
    }

    fn neighbors(&self, pos: Pos) -> Vec<(Pos, Cell)> {
        let Pos(col, row) = pos;
        let r: Vec<(Pos, Cell)> = NEIGHBORS
            .iter()
            .filter_map(|(c, r)| {
                self.get(Pos(col + c, row + r))
                    .map(|cell| (Pos(col + c, row + r), cell))
            })
            .collect();

        r
    }

    fn solve(&mut self) -> Result<(bool, f32)> {
        let mut active: Vec<Pos> = Vec::new();
        let mut luck = 1f32;

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
                    Cell::Mine => return Ok((false, luck)),
                    _ => (),
                }
            }

            // Already done
            if self.unknowns == 0 {
                break;
            }

            let remaining_mines = self.minefield.number_of_mines() - self.flags;

            // Uncover remaining cells when all mines are flagged, then we are done
            if remaining_mines == 0 {
                for col in 0..self.minefield.width() {
                    for row in 0..self.minefield.height() {
                        let pos = Pos(col, row);
                        if let Some(Cell::Unknown) = self.get(pos) {
                            self.uncover(pos)?;
                        }
                    }
                }
                break;
            }

            if new_info {
                continue;
            }

            // Simple algo didn't find new info, try heavier iterative algo now.

            let naive_chance = remaining_mines as f32 / self.unknowns as f32;

            let mut probs: HashMap<Pos, f32> = HashMap::new();
            for pos in active.iter().copied() {
                let neighbors = self.neighbors(pos);
                probs.extend(neighbors.iter().filter_map(|(pos, cell)| {
                    matches!(cell, Cell::Unknown).then(|| (*pos, naive_chance))
                }));
            }

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
                        let correction = (expected - sum) / unknowns.len() as f32;

                        max_correction_diff = f32::max(max_correction_diff, f32::abs(correction));

                        for pos in unknowns {
                            if let Some(p) = probs.get_mut(&pos) {
                                *p = f32::clamp(*p + correction, 0f32, 1f32);
                            }
                        }
                    }
                }

                // Reduce total probability if it is more then the remaining mines
                let sum: f32 = probs.iter().map(|(_, p)| p).copied().sum();
                if sum > remaining_mines as f32 {
                    let correction = (remaining_mines as f32 - sum) / probs.len() as f32;
                    for (_, p) in probs.iter_mut() {
                        *p = f32::clamp(*p + correction, 0f32, 1f32);
                    }
                    max_correction_diff = f32::max(max_correction_diff, f32::abs(correction));
                }

                // Enough conversion, done iterating
                if max_correction_diff < 0.0001 {
                    break;
                }
            }

            let sum: f32 = probs.iter().map(|(_, p)| p).copied().sum();
            let border_unknowns: i32 = probs.len().try_into().unwrap();
            let isolated_unknowns: i32 = self.unknowns - border_unknowns;
            let p_other = (remaining_mines as f32 - sum) / (isolated_unknowns as f32);

            let best_guess = probs
                .iter()
                .min_by(|(_, p1), (_, p2)| (*p1).partial_cmp(*p2).unwrap());

            // Lazy
            let pos_other = || {
                for col in 0..self.minefield.width() {
                    for row in 0..self.minefield.height() {
                        let pos = Pos(col, row);
                        if let Some(Cell::Unknown) = self.get(pos) {
                            if probs.get(&pos).is_none() {
                                return pos;
                            }
                        }
                    }
                }
                panic!();
            };

            let best_guess = match best_guess {
                Some((_, p)) if isolated_unknowns > 0 && p_other < *p => (pos_other(), p_other),
                Some((pos, p)) => (*pos, *p),
                None => (pos_other(), p_other),
            };

            luck *= 1f32 - best_guess.1;

            let pos = best_guess.0;
            let cell = self.uncover(pos)?;
            if let Cell::Mine = cell {
                return Ok((false, luck));
            }
            next.push(pos);
        }

        Ok((self.solved(), luck))
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
        unknowns == 0 && mines == 0 && flags == self.minefield.number_of_mines()
    }

    fn show(&self) {
        for row in 0..self.minefield.height() {
            for col in 0..self.minefield.width() {
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

    #[clap(short, long, value_parser)]
    native: bool,
}

fn body<T, M>(cli: Cli, new: T) -> Result<()>
where
    T: Fn(Mode) -> Result<M>,
    M: Minefield,
{
    if let Some(iterations) = cli.iterations {
        let mut success = 0;
        let mut luck_sum = 0f32;
        for _ in 0..iterations {
            let mut minefield = new(cli.mode)?;
            let mut solver = Solver::new(&mut minefield)?;
            if let (true, luck) = solver.solve()? {
                success += 1;
                luck_sum += luck;
            }
        }

        println!(
            "Solved {}/{} successful ({}), {:?}, avg luck {}",
            success,
            iterations,
            success as f32 / iterations as f32,
            cli.mode,
            luck_sum / success as f32
        );
    } else {
        let mut minefield = new(cli.mode)?;
        let mut solver = Solver::new(&mut minefield)?;

        let (solved, luck) = solver.solve()?;
        solver.show();

        println!();
        println!("Solved: {}, luck: {}", solved, luck);
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.native {
        body(cli, |mode: Mode| -> Result<_> {
            Ok(RustMinefield::new(mode))
        })
    } else {
        Python::with_gil(|py| {
            let builder = MinefieldBuilder::new(py)?;
            body(cli, |mode: Mode| builder.build(mode))
        })
    }
}

#[test]
fn bla() -> Result<()> {
    let mut minefield = RustMinefield {
        field: vec![
            false, false, false, false, false, false, true, false, false, false, false, false,
            true, false, false, true,
        ],
        width: 4,
        height: 4,
        number_of_mines: 3,
    };

    let mut solver = Solver::new(&mut minefield)?;

    solver.solve()?;
    assert!(solver.solved());

    Ok(())
}
