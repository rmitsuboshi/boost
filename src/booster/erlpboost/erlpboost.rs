//! This file defines `ERLPBoost` based on the paper
//! "Entropy Regularized LPBoost"
//! by Warmuth et al.
//! 
use crate::{
    Sample,
    Booster,
    WeakLearner,

    State,
    Classifier,
    CombinedHypothesis,
    common::utils,
    common::checker,
    research::Research,
};

use super::qp_model::QPModel;


use std::cell::RefCell;



/// ERLPBoost struct. 
/// This code is based on this paper: [Entropy Regularized LPBoost](https://link.springer.com/chapter/10.1007/978-3-540-87987-9_23) by Manfred K. Warmuth, Karen A. Glocer, and S. V. N. Vishwanathan.
/// 
/// # Example
/// The following code shows a small example 
/// for running [`ERLPBoost`](ERLPBoost).  
/// See also:
/// - [`ERLPBoost::nu`]
/// - [`DTree`]
/// - [`DTreeClassifier`]
/// - [`CombinedHypothesis<F>`]
/// 
/// [`ERLPBoost::nu`]: ERLPBoost::nu
/// [`DTree`]: crate::weak_learner::DTree
/// [`DTreeClassifier`]: crate::weak_learner::DTreeClassifier
/// [`CombinedHypothesis<F>`]: crate::hypothesis::CombinedHypothesis
/// 
/// 
/// ```no_run
/// use miniboosts::prelude::*;
/// 
/// // Read the training sample from the CSV file.
/// // We use the column named `class` as the label.
/// let has_header = true;
/// let mut sample = Sample::from_csv(path_to_csv_file, has_header)
///     .unwrap()
///     .set_target("class");
/// 
/// 
/// // Get the number of training examples.
/// let n_sample = sample.shape().0 as f64;
/// 
/// // Set the upper-bound parameter of outliers in `sample`.
/// // Here we assume that the outliers are at most 1% of `sample`.
/// let nu = 0.1 * n_sample;
/// 
/// // Initialize `ERLPBoost` and set the tolerance parameter as `0.01`.
/// // This means `booster` returns a hypothesis whose training error is
/// // less than `0.01` if the traing examples are linearly separable.
/// // Note that the default tolerance parameter is set as `1 / n_sample`,
/// // where `n_sample = sample.shape().0` is 
/// // the number of training examples in `sample`.
/// let booster = ERLPBoost::init(&sample)
///     .tolerance(0.01)
///     .nu(0.1 * n_sample);
/// 
/// // Set the weak learner with setting parameters.
/// let weak_learner = DecisionTree::init(&sample)
///     .max_depth(2)
///     .criterion(Criterion::Edge);
/// 
/// // Run `ERLPBoost` and obtain the resulting hypothesis `f`.
/// let f: CombinedHypothesis<DTreeClassifier> = booster.run(&weak_learner);
/// 
/// // Get the predictions on the training set.
/// let predictions: Vec<i64> = f.predict_all(&sample);
/// 
/// // Calculate the training loss.
/// let target = sample.target();
/// let training_loss = target.into_iter()
///     .zip(predictions)
///     .map(|(&y, fx) if y as i64 == fx { 0.0 } else { 1.0 })
///     .sum::<f64>()
///     / n_sample;
/// 
///
/// println!("Training Loss is: {training_loss}");
/// ```
pub struct ERLPBoost<'a, F> {
    // Training sample
    sample: &'a Sample,

    // Distribution over examples
    dist: Vec<f64>,

    // `gamma_hat` corresponds to $\min_{q=1, .., t} P^q (d^{q-1})$
    gamma_hat: f64,

    // `gamma_star` corresponds to $P^{t-1} (d^{t-1})$
    gamma_star: f64,
    // regularization parameter defined in the paper
    eta: f64,

    half_tolerance: f64,

    qp_model: Option<RefCell<QPModel>>,

    hypotheses: Vec<F>,
    weights: Vec<f64>,


    // an accuracy parameter for the sub-problems
    n_sample: usize,
    nu: f64,


    terminated: usize,

    max_iter: usize,
}


impl<'a, F> ERLPBoost<'a, F> {
    /// Initialize the `ERLPBoost`.
    /// Use `data` for argument.
    /// This method does not care 
    /// whether the label is included in `data` or not.
    pub fn init(sample: &'a Sample) -> Self {
        let n_sample = sample.shape().0;
        assert!(n_sample != 0);


        // Set uni as an uniform weight
        let uni = 1.0 / n_sample as f64;

        // Compute $\ln(n_sample)$ in advance
        let ln_n_sample = (n_sample as f64).ln();


        // Set tolerance
        let half_tolerance = uni / 2.0;


        // Set regularization parameter
        let eta = 0.5_f64.max(ln_n_sample / half_tolerance);

        // Set gamma_hat and gamma_star
        let gamma_hat  = 1.0;
        let gamma_star = f64::MIN;


        ERLPBoost {
            sample,

            dist: vec![uni; n_sample],
            gamma_hat,
            gamma_star,
            eta,
            half_tolerance,
            qp_model: None,

            hypotheses: Vec::new(),
            weights: Vec::new(),


            n_sample,
            nu: 1.0,

            terminated: usize::MAX,
            max_iter: usize::MAX,
        }
    }


    fn init_solver(&mut self) {
        checker::check_nu(self.nu, self.n_sample);


        let upper_bound = 1.0 / self.nu;
        let qp_model = RefCell::new(QPModel::init(
            self.eta, self.n_sample, upper_bound
        ));

        self.qp_model = Some(qp_model);
    }


    /// Updates the capping parameter.
    pub fn nu(mut self, nu: f64) -> Self {
        assert!(1.0 <= nu && nu <= self.n_sample as f64);
        self.nu = nu;
        self.regularization_param();

        self
    }


    /// Returns the break iteration.
    /// This method returns `0` before the `.run()` call.
    #[inline(always)]
    pub fn terminated(&self) -> usize {
        self.terminated
    }


    /// Set the tolerance parameter.
    #[inline(always)]
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.half_tolerance = tolerance / 2.0;
        self
    }


    /// Setter method of `self.eta`
    #[inline(always)]
    fn regularization_param(&mut self) {
        let ln_n_sample = (self.n_sample as f64 / self.nu).ln();


        self.eta = 0.5_f64.max(ln_n_sample / self.half_tolerance);
    }


    /// `max_loop` returns the maximum iteration
    /// of the Adaboost to find a combined hypothesis
    /// that has error at most `tolerance`.
    fn max_loop(&mut self) -> usize {
        let n_sample = self.n_sample as f64;

        let mut max_iter = 4.0 / self.half_tolerance;


        let ln_n_sample = (n_sample / self.nu).ln();
        let temp = 8.0 * ln_n_sample / self.half_tolerance.powi(2);


        max_iter = max_iter.max(temp);

        max_iter.ceil() as usize
    }
}


impl<F> ERLPBoost<'_, F>
    where F: Classifier
{
    /// Update `self.gamma_hat`.
    /// `self.gamma_hat` holds the minimum value of the objective value.
    #[inline]
    fn update_gamma_hat_mut(&mut self, h: &F)
    {
        let edge = utils::edge_of_hypothesis(self.sample, &self.dist[..], h);
        let entropy = utils::entropy_from_uni_distribution(&self.dist[..]);

        let obj_val = edge + (entropy / self.eta);

        self.gamma_hat = self.gamma_hat.min(obj_val);
    }


    /// Update `self.gamma_star`.
    /// `self.gamma_star` holds the current optimal value.
    fn update_gamma_star_mut(&mut self)
    {
        let max_edge = self.hypotheses.iter()
            .map(|h|
                utils::edge_of_hypothesis(self.sample, &self.dist, h)
            )
            .reduce(f64::max)
            .unwrap();
        let entropy = utils::entropy_from_uni_distribution(&self.dist);
        self.gamma_star = max_edge + (entropy / self.eta);
    }


    /// Updates `self.dist`
    /// This method repeatedly minimizes the quadratic approximation of 
    /// ERLPB. objective around current distribution `self.dist`.
    /// Then update `self.dist` as the optimal solution of 
    /// the approximate problem. 
    /// This method continues minimizing the quadratic objective 
    /// while the decrease of the optimal value is 
    /// greater than `self.sub_tolerance`.
    fn update_distribution_mut(&mut self, clf: &F)
    {
        self.qp_model.as_ref()
            .unwrap()
            .borrow_mut()
            .update(self.sample, &mut self.dist[..], clf);

        self.dist = self.qp_model.as_ref()
            .unwrap()
            .borrow()
            .distribution();
    }
}


impl<F> Booster<F> for ERLPBoost<'_, F>
    where F: Classifier + Clone,
{
    fn preprocess<W>(
        &mut self,
        _weak_learner: &W,
    )
        where W: WeakLearner<Hypothesis = F>
    {
        let n_sample = self.sample.shape().0;
        let uni = 1.0 / n_sample as f64;

        self.dist = vec![uni; n_sample];

        self.max_iter = self.max_loop();
        self.terminated = self.max_iter;

        self.hypotheses = Vec::new();

        self.gamma_hat = 1.0;
        self.gamma_star = -1.0;


        assert!((0.0..1.0).contains(&self.half_tolerance));
        self.regularization_param();
        self.init_solver();
    }


    fn boost<W>(
        &mut self,
        weak_learner: &W,
        iteration: usize,
    ) -> State
        where W: WeakLearner<Hypothesis = F>,
    {
        if self.max_iter < iteration {
            return State::Terminate;
        }

        // Receive a hypothesis from the base learner
        let h = weak_learner.produce(self.sample, &self.dist[..]);


        // update `self.gamma_hat`
        self.update_gamma_hat_mut(&h);


        // Check the stopping criterion
        let diff = self.gamma_hat - self.gamma_star;
        if diff <= self.half_tolerance {
            self.terminated = iteration;
            return State::Terminate;
        }

        // At this point, the stopping criterion is not satisfied.

        // Update the parameters
        self.update_distribution_mut(&h);


        // Append a new hypothesis to `clfs`.
        self.hypotheses.push(h);


        // update `self.gamma_star`.
        self.update_gamma_star_mut();

        State::Continue
    }


    fn postprocess<W>(
        &mut self,
        _weak_learner: &W,
    ) -> CombinedHypothesis<F>
        where W: WeakLearner<Hypothesis = F>
    {
        self.weights = self.qp_model.as_ref()
            .unwrap()
            .borrow_mut()
            .weight()
            .collect::<Vec<_>>();

        CombinedHypothesis::from_slices(&self.weights[..], &self.hypotheses[..])
    }
}

impl<H> Research<H> for ERLPBoost<'_, H>
    where H: Classifier + Clone,
{
    fn current_hypothesis(&self) -> CombinedHypothesis<H> {
        let weights = self.qp_model.as_ref()
            .unwrap()
            .borrow_mut()
            .weight()
            .collect::<Vec<_>>();

        CombinedHypothesis::from_slices(&weights[..], &self.hypotheses[..])
    }
}

