//! This file defines `LPBoost` based on the paper
//! ``Boosting algorithms for Maximizing the Soft Margin''
//! by Warmuth et al.
//! 
#[cfg(not(feature="gurobi"))]
use super::lp_model::LPModel;

#[cfg(feature="gurobi")]
use super::gurobi_lp_model::LPModel;

use crate::{
    Sample,
    Booster,
    WeakLearner,

    Classifier,
    WeightedMajority,
    common::utils,
    common::checker,
    research::Research,
};


use std::cell::RefCell;
use std::ops::ControlFlow;


/// The `LPBoost` algorithm 
/// proposed by Demiriz, Bennett, and Shawe-Taylor.  
/// `LPBoost` is originally proposed in the following paper:  
/// 
/// [Ayhan Demiriz, Kristin P. Bennett, and John Shawe-Taylor - Linear Programming Boosting via Column Generation](https://www.researchgate.net/publication/220343627_Linear_Programming_Boosting_via_Column_Generation)
/// 
/// My implementation of `LPBoost` is based on the following paper:
/// 
/// [Manfred K. Warmuth, Karen Glocer, and Gunnar Rätsch - Boosting algorithms for Maximizing the Soft Margin](https://proceedings.neurips.cc/paper/2007/file/cfbce4c1d7c425baf21d6b6f2babe6be-Paper.pdf)
/// 
/// Given a set `{(x_{1}, y_{1}), (x_{2}, y_{2}), ..., (x_{m}, y_{m})}`
/// of training examples,
/// a capping parameters `ν ∈ [1, m]`, and
/// an accuracy parameter `ε > 0`,
/// `LPBoost` aims to find an `ε`-approximate solution of
/// the soft-margin optimization problem:
/// ```txt
///  max  ρ - (1/ν) Σ_{i=1}^{m} ξ_{i}
/// ρ,w,ξ
/// s.t. y_{i} Σ_{h ∈ Δ_{H}} w_{h} h(x_{i}) ≥ ρ - ξ_{i},
///                                         for all i ∈ [m],
///      w ∈ Δ_{H},
///      ξ ≥ 0.
/// ```
/// 
/// # Convergence rate
/// There exists a training set of size `m > 0` such that
/// `LPBoost` takes `Ω( m )` iterations for the worst case.
///
///
/// # Related information
/// - Currently (2023), `LPBoost` has no convergence guarantee.
/// - [`ERLPBoost`](crate::booster::ERLPBoost), 
/// A stabilized version of `LPBoost` is 
/// proposed by Warmuth et al. (2008).
/// 
/// # Example
/// The following code shows a small example for running [`LPBoost`].  
/// 
/// 
/// ```no_run
/// use miniboosts::prelude::*;
/// 
/// // Read the training sample from the CSV file.
/// // We use the column named `class` as the label.
/// let sample = SampleReader::new()
///     .file(path_to_file)
///     .has_header(true)
///     .target_feature("class")
///     .read()
///     .unwrap();
/// 
/// 
/// // Get the number of training examples.
/// let n_sample = sample.shape().0 as f64;
/// 
/// // Set the upper-bound parameter of outliers in `sample`.
/// // Here we assume that the outliers are at most 10% of `sample`.
/// let nu = 0.1 * n_sample;
/// 
/// // Initialize `LPBoost` and set the tolerance parameter as `0.01`.
/// // This means `booster` returns a hypothesis whose training error is
/// // less than `0.01` if the traing examples are linearly separable.
/// let mut booster = LPBoost::init(&sample)
///     .tolerance(0.01)
///     .nu(0.1 * n_sample);
/// 
/// // Set the weak learner with setting parameters.
/// let weak_learner = DecisionTreeBuilder::new(&sample)
///     .max_depth(2)
///     .criterion(Criterion::Entropy)
///     .build();
/// 
/// // Run `LPBoost` and obtain the resulting hypothesis `f`.
/// let f = booster.run(&weak_learner);
/// 
/// // Get the predictions on the training set.
/// let predictions = f.predict_all(&sample);
/// 
/// // Calculate the training loss.
/// let target = sample.target();
/// let training_loss = target.into_iter()
///     .zip(predictions)
///     .map(|(&y, fx)| if y as i64 == fx { 0.0 } else { 1.0 })
///     .sum::<f64>()
///     / n_sample;
/// 
///
/// println!("Training Loss is: {training_loss}");
/// ```
pub struct LPBoost<'a, F> {
    // Training sample
    sample: &'a Sample,

    // Distribution over examples
    dist: Vec<f64>,

    // min-max edge of the new hypothesis
    gamma_hat: f64,

    // Tolerance parameter
    tolerance: f64,


    // Number of examples
    n_sample: usize,


    // Capping parameter
    nu: f64,


    // GRBModel.
    lp_model: Option<RefCell<LPModel>>,


    hypotheses: Vec<F>,
    weights: Vec<f64>,


    terminated: usize,
}


impl<'a, F> LPBoost<'a, F>
    where F: Classifier
{
    /// Constructs a new instance of `LPBoost`.
    /// 
    /// Time complexity: `O(1)`.
    pub fn init(sample: &'a Sample) -> Self {
        let n_sample = sample.shape().0;


        let uni = 1.0 / n_sample as f64;
        Self {
            sample,

            dist: Vec::new(),
            gamma_hat: 1.0,
            tolerance: uni,
            n_sample,
            nu: 1.0,
            lp_model: None,

            hypotheses: Vec::new(),
            weights: Vec::new(),


            terminated: usize::MAX,
        }
    }


    /// This method updates the capping parameter.
    /// This parameter must be in `[1, # of training examples]`.
    /// 
    /// Time complexity: `O(1)`.
    pub fn nu(mut self, nu: f64) -> Self {
        checker::check_nu(nu, self.n_sample);
        self.nu = nu;

        self
    }


    /// Initializes the LP solver.
    fn init_solver(&mut self) {
        let n_sample = self.sample.shape().0 as f64;
        assert!((1.0..=n_sample).contains(&self.nu));

        let upper_bound = 1.0 / self.nu;

        let lp_model = RefCell::new(LPModel::init(self.n_sample, upper_bound));

        self.lp_model = Some(lp_model);
    }


    /// Set the tolerance parameter.
    /// LPBoost guarantees the `tolerance`-approximate solution to
    /// the soft margin optimization.  
    /// Default value is `0.01`.
    /// 
    /// Time complexity: `O(1)`.
    #[inline(always)]
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }


    /// Returns the terminated iteration.
    /// This method returns `usize::MAX` before the boosting step.
    /// 
    /// Time complexity: `O(1)`.
    #[inline(always)]
    pub fn terminated(&self) -> usize {
        self.terminated
    }


    /// This method updates `self.dist` and `self.gamma_hat`
    /// by solving a linear program
    /// over the hypotheses obtained in past rounds.
    /// 
    /// Time complexity depends on the LP solver.
    #[inline(always)]
    fn update_distribution_mut(&self, h: &F) -> f64
    {
        self.lp_model.as_ref()
            .expect("Failed to call `.as_ref()` to `self.lp_model`")
            .borrow_mut()
            .update(self.sample, h)
    }
}


impl<F> Booster<F> for LPBoost<'_, F>
    where F: Classifier + Clone,
{
    type Output = WeightedMajority<F>;


    fn name(&self) -> &str {
        "LPBoost"
    }


    fn info(&self) -> Option<Vec<(&str, String)>> {
        let (n_sample, n_feature) = self.sample.shape();
        let ratio = self.nu * 100f64 / n_sample as f64;
        let nu = utils::format_unit(self.nu);
        let info = Vec::from([
            ("# of examples", format!("{n_sample}")),
            ("# of features", format!("{n_feature}")),
            ("Tolerance", format!("{}", self.tolerance)),
            ("Max iteration", format!("-")),
            ("Capping (outliers)", format!("{nu} ({ratio: >7.3} %)"))
        ]);
        Some(info)
    }


    fn preprocess<W>(
        &mut self,
        _weak_learner: &W,
    )
        where W: WeakLearner<Hypothesis = F>
    {
        self.sample.is_valid_binary_instance();
        let n_sample = self.sample.shape().0;
        let uni = 1.0_f64 / self.n_sample as f64;

        self.init_solver();

        self.n_sample = n_sample;
        self.dist = vec![uni; n_sample];
        self.gamma_hat = 1.0;
        self.hypotheses = Vec::new();
        self.terminated = usize::MAX;
    }


    fn boost<W>(
        &mut self,
        weak_learner: &W,
        iteration: usize,
    ) -> ControlFlow<usize>
        where W: WeakLearner<Hypothesis = F>,
    {
        let h = weak_learner.produce(self.sample, &self.dist);

        // Each element in `margins` is the product of
        // the predicted vector and the correct vector
        let ghat = utils::edge_of_hypothesis(self.sample, &self.dist[..], &h);

        self.gamma_hat = ghat.min(self.gamma_hat);

        let gamma_star = self.update_distribution_mut(&h);


        if gamma_star >= self.gamma_hat - self.tolerance {
            self.hypotheses.push(h);
            self.terminated = self.hypotheses.len();
            return ControlFlow::Break(iteration);
        }

        self.hypotheses.push(h);

        // Update the distribution over the training examples.
        self.dist = self.lp_model.as_ref()
            .expect("Failed to call `.as_ref()` to `self.lp_model`")
            .borrow()
            .distribution();

        ControlFlow::Continue(())
    }


    fn postprocess<W>(
        &mut self,
        _weak_learner: &W,
    ) -> Self::Output
        where W: WeakLearner<Hypothesis = F>
    {
        self.weights = self.lp_model.as_ref()
            .expect("Failed to call `.as_ref()` to `self.lp_model`")
            .borrow()
            .weight()
            .collect::<Vec<_>>();

        WeightedMajority::from_slices(&self.weights[..], &self.hypotheses[..])
    }
}


impl<H> Research for LPBoost<'_, H>
    where H: Classifier + Clone,
{
    type Output = WeightedMajority<H>;
    fn current_hypothesis(&self) -> Self::Output {
        let weights = self.lp_model.as_ref()
            .expect("Failed to call `.as_ref()` to `self.lp_model`")
            .borrow()
            .weight()
            .collect::<Vec<_>>();

        WeightedMajority::from_slices(&weights[..], &self.hypotheses[..])
    }
}
