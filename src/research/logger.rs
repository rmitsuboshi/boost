use colored::Colorize;

use crate::{
    Sample,
    Booster,
    WeakLearner,
    Classifier,
};
use super::ObjectiveFunction;

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::time::Instant;
use std::ops::ControlFlow;

const DEFAULT_ROUND: usize = 100;
const DEFAULT_TIMELIMIT_MILLIS: u128 = u128::MAX;
const WIDTH: usize = 9;
const PREC_WIDTH: usize = 6;
const TIME_WIDTH: usize = 6;
const FULL_WIDTH: usize = 50;
const STAT_WIDTH: usize = (FULL_WIDTH - 4) / 2;
const HEADER: &str = "ObjectiveValue,TrainLoss,TestLoss,Time\n";


/// Struct `Logger` provides a generic function that
/// logs objective value, train/test loss value, and running time
/// for each step of boosting.
pub struct Logger<'a, B, W, F, G> {
    booster: B,
    weak_learner: W,
    objective_func: F,
    loss_func: G,
    train: &'a Sample,
    test: &'a Sample,
    time_limit: u128,
    round: usize,
}


impl<'a, B, W, F, G> Logger<'a, B, W, F, G> {
    /// Create a new instance of `Logger`.
    pub fn new(
        booster: B,
        weak_learner: W,
        objective_func: F,
        loss_func: G,
        train: &'a Sample,
        test: &'a Sample,
    ) -> Self
    {
        Self {
            booster,
            weak_learner,
            loss_func,
            objective_func,
            train,
            test,
            time_limit: DEFAULT_TIMELIMIT_MILLIS,
            round: DEFAULT_ROUND,
        }
    }
}

impl<'a, H, B, W, F, G, O> Logger<'a, B, W, F, G>
    where B: Booster<H, Output=O> + Research<Output=O>,
          O: Classifier,
          W: WeakLearner<Hypothesis = H>,
          F: ObjectiveFunction<O>,
          G: Fn(&Sample, &O) -> f64,
{
    /// Set the time limit for boosting algorithm as milliseconds.
    /// If the boosting algorithm reaches this limit,
    /// breaks immediately.
    #[inline(always)]
    pub fn time_limit_as_millis(mut self, time_limit: u128) -> Self {
        self.time_limit = time_limit;
        self
    }


    /// Set the time limit for boosting algorithm as seconds.
    /// If the boosting algorithm reaches this limit,
    /// breaks immediately.
    #[inline(always)]
    pub fn time_limit_as_secs(mut self, time_limit: u64) -> Self {
        self.time_limit = (time_limit as u128).checked_mul(1_000_u128)
            .expect("The time limit (ms) cannot be represented as u128");
        self
    }


    /// Set the time limit for boosting algorithm as minutes.
    /// If the boosting algorithm reaches this limit,
    /// breaks immediately.
    #[inline(always)]
    pub fn time_limit_as_mins(mut self, time_limit: u64) -> Self {
        self.time_limit = (time_limit as u128).checked_mul(60_u128)
            .expect("The time limit (s) cannot be represented as u128")
            .checked_mul(1_000u128)
            .expect("The time limit (ms) cannot be represented as u128");
        self
    }


    #[inline(always)]
    fn print_log_header(&self) {
        println!(
            "{} {:>WIDTH$}\t\t{:>WIDTH$}\t{:>WIDTH$}\t{:>WIDTH$}\t{:>WIDTH$}",
            "     ",
            "".bold().red(),
            "OBJECTIVE".bold().blue(),
            "TRAIN".bold().green(),
            "TEST".bold().yellow(),
            "ACC.".bold().cyan(),
        );
        println!(
            "{} {:>WIDTH$}\t\t{:>WIDTH$}\t{:>WIDTH$}\t{:>WIDTH$}\t{:>WIDTH$}\n",
            "     ",
            "ROUND".bold().red(),
            "VALUE".bold().blue(),
            "ERROR".bold().green(),
            "ERROR".bold().yellow(),
            "TIME".bold().cyan(),
        );
    }


    #[inline(always)]
    fn print_stats(&self) {
        let limit = if self.time_limit != u128::MAX {
            format!("{} s", (self.time_limit / 1000))
        } else {
            "Nothing".into()
        };
        println!(
            "\n\
            {:=>FULL_WIDTH$}\n\
            {:^FULL_WIDTH$}\n\
            {:=>FULL_WIDTH$}\n\
            {:>STAT_WIDTH$}\t{:<STAT_WIDTH$}\n\
            {:>STAT_WIDTH$}\t{:<STAT_WIDTH$}\n\
            {:>STAT_WIDTH$}\t{:<STAT_WIDTH$}\n\
            {:>STAT_WIDTH$}\t{:<STAT_WIDTH$}\n\
            {:=>FULL_WIDTH$}\n\
            ",
            "".bold(),
            "STATS".bold(),
            "".bold(),
            "BOOSTER",
            self.booster.name().bold().green(),
            "WEAK LEARNER",
            self.weak_learner.name().bold().green(),
            "OBJECTIVE",
            self.objective_func.name().bold().green(),
            "TIME LIMIT",
            limit.bold().green(),
            "".bold(),
        );
    }


    /// Set the interval to print the current status.
    /// By default, the method `run` prints its status every `100` rounds.
    /// If you don't want to print the log,
    /// set `usize::MAX`.
    #[inline(always)]
    pub fn print_every(mut self, round: usize) -> Self {
        self.round = round;
        self
    }



    /// Run the given boosting algorithm with logging.
    /// Note that this method is almost the same as `Booster::run`.
    /// This method measures running time per iteration.
    #[inline(always)]
    pub fn run<P: AsRef<Path>>(&mut self, filename: P)
        -> std::io::Result<O>
    {
        // Open file
        let mut file = File::create(filename)?;

        // Write header to the file
        file.write_all(HEADER.as_bytes())?;

        // ---------------------------------------------------------------------
        // Pre-processing
        self.booster.preprocess(&self.weak_learner);
        self.print_stats();


        // Cumulative time
        let mut time_acc = 0;

        // ---------------------------------------------------------------------
        // Boosting step
        if self.round != usize::MAX { self.print_log_header(); }
        (1..).try_for_each(|iter| {
            // Start measuring time
            let now = Instant::now();

            let flow = self.booster.boost(&self.weak_learner, iter);

            // Stop measuring and convert `Duration` to Milliseconds.
            let time = now.elapsed().as_millis();

            // Update the cumulative time
            time_acc += time;

            let hypothesis = self.booster.current_hypothesis();

            let obj = self.objective_func.eval(self.train, &hypothesis);
            let train = (self.loss_func)(self.train, &hypothesis);
            let test = (self.loss_func)(self.test, &hypothesis);

            // Write the results to `file`.
            let line = format!("{obj},{train},{test},{time_acc}\n");
            file.write_all(line.as_bytes())
                .expect("Failed to writing {filename:?}");

            if time_acc > self.time_limit {
                println!(
                    "{} {}\t\t{}\t{}\t{}\t{}\n",
                    "[TLE]".bold().bright_red(),
                    format!("{:>WIDTH$}", iter).bold().red(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", obj).bold().blue(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", train).bold().green(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", test).bold().yellow(),
                    format!("{:>TIME_WIDTH$} ms", time_acc).bold().cyan(),
                );
                return ControlFlow::Break(iter);
            }


            if self.round != usize::MAX && iter % self.round == 0 {
                println!(
                    "{} {}\t\t{}\t{}\t{}\t{}",
                    "[LOG]".bold().magenta(),
                    format!("{:>WIDTH$}", iter).red(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", obj).blue(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", train).green(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", test).yellow(),
                    format!("{:>TIME_WIDTH$} ms", time_acc).cyan(),
                );
            }


            if flow.is_break() && self.round != usize::MAX {
                println!(
                    "{} {}\t\t{}\t{}\t{}\t{}\n",
                    "[FIN]".bold().bright_green(),
                    format!("{:>WIDTH$}", iter).red(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", obj).bold().blue(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", train).bold().green(),
                    format!("{:>WIDTH$.PREC_WIDTH$}", test).bold().yellow(),
                    format!("{:>TIME_WIDTH$} ms", time_acc).bold().cyan(),
                );
            }
            flow
        });


        let f = self.booster.postprocess(&self.weak_learner);
        Ok(f)
    }
}


/// Implementing this trait allows you to use `Logger` to
/// log algorithm's behavor.
pub trait Research {
    /// The combined hypothesis at middle stages.
    /// In general, this type is the same as the one in `Booster`.
    type Output;


    /// Returns the combined hypothesis at current state.
    fn current_hypothesis(&self) -> Self::Output;
}


