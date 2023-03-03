//! Provides [`GBM`](GBM) by Friedman, 2001.
use polars::prelude::*;


use crate::{
    common::loss_functions::*,
    Booster,
    WeakLearner,
    State,
    Regressor,
    CombinedHypothesis
};

// use crate::research::Logger;


/// Defines `GBM`.
/// This struct is based on the book: 
/// [Greedy Function Approximation: A Gradient Boosting Machine](https://projecteuclid.org/journals/annals-of-statistics/volume-29/issue-5/Greedy-function-approximation-A-gradient-boostingmachine/10.1214/aos/1013203451.full)
/// by Jerome H. Friedman, 2001.
/// 
/// # Example
/// The following code shows a small example 
/// for running [`GBM`](GBM).  
/// See also:
/// - [`Regressor`]
/// - [`CombinedHypothesis<F>`]
/// - [`DataFrame`]
/// - [`Series`]
/// - [`CsvReader`]
/// 
/// [`RTree`]: crate::weak_learner::RTree
/// [`RTreeRegressor`]: crate::weak_learner::RTreeRegressor
/// [`CombinedHypothesis<F>`]: crate::hypothesis::CombinedHypothesis
/// [`RTree::max_depth`]: crate::weak_learner::RTree::max_depth
/// [`RTree::criterion`]: crate::weak_learner::RTree::criterion
/// [`DataFrame`]: polars::prelude::DataFrame
/// [`Series`]: polars::prelude::Series
/// [`DataFrame::shape`]: polars::prelude::DataFrame::shape
/// [`CsvReader`]: polars::prelude::CsvReader
/// 
/// 
/// ```no_run
/// use polars::prelude::*;
/// use miniboosts::prelude::*;
/// 
/// // Read the training data from the CSV file.
/// let mut data = CsvReader::from_path(path_to_csv_file)
///     .unwrap()
///     .has_header(true)
///     .finish()
///     .unwrap();
/// 
/// // Split the column corresponding to labels.
/// let target = data.drop_in_place(class_column_name).unwrap();
/// 
/// // Initialize `GBM` and set the tolerance parameter as `0.01`.
/// // This means `booster` returns a hypothesis whose training error is
/// // less than `0.01` if the traing examples are linearly separable.
/// // Note that the default tolerance parameter is set as `1 / n_sample`,
/// // where `n_sample = data.shape().0` is 
/// // the number of training examples in `data`.
/// let booster = GBM::init(&data, &target)
///     .loss(GBMLoss::L1);
/// 
/// // Set the weak learner with setting parameters.
/// let weak_learner = RTree::init(&data, &target)
///     .max_depth(1)
///     .loss_type(LossType::L1);
/// 
/// // Run `GBM` and obtain the resulting hypothesis `f`.
/// let f: CombinedHypothesis<RTreeRegressor> = booster.run(&weak_learner);
/// 
/// // Get the predictions on the training set.
/// let predictions: Vec<f64> = f.predict_all(&data);
/// 
/// // Get the number of training examples.
/// let n_sample = data.shape().0 as f64;
/// 
/// // Calculate the training loss.
/// let training_loss = target.f64()
///     .unwrap()
///     .into_iter()
///     .zip(predictions)
///     .map(|(true_label, prediction) {
///         let true_label = true_label.unwrap();
///         (true_label - prediction).abs()
///     })
///     .sum::<f64>()
///     / n_sample;
/// 
///
/// println!("Training Loss is: {training_loss}");
/// ```
pub struct GBM<'a, F> {
    // Training data
    data: &'a DataFrame,


    // Correponding label
    target: &'a Series,


    // Modified labels
    residuals: Vec<f64>,

    // Distribution on examples.
    // Since GBM does not maintain a distribution over examples,
    // we use all-one vector.
    ones: Vec<f64>,

    // Tolerance parameter
    tolerance: f64,

    // Weights on hypotheses
    weights: Vec<f64>,

    // Hypohteses obtained by the weak-learner.
    classifiers: Vec<F>,


    // Some struct that implements `LossFunction` trait
    loss: GBMLoss,


    // Max iteration until GBM guarantees the optimality.
    max_iter: usize,

    // Terminated iteration.
    // GBM terminates in eary step 
    // if the training set is linearly separable.
    terminated: usize,
}




impl<'a, F> GBM<'a, F>
{
    /// Initialize the `GBM`.
    /// This method sets some parameters `GBM` holds.
    pub fn init(data: &'a DataFrame, target: &'a Series) -> Self {
        assert!(!data.is_empty());

        let n_sample = data.shape().0;


        Self {
            tolerance: 0.0,

            weights: Vec::new(),
            classifiers: Vec::new(),

            data,
            target,

            residuals: Vec::with_capacity(n_sample),
            ones: vec![1.0; n_sample],

            loss: GBMLoss::L2,

            max_iter: 100,

            terminated: usize::MAX,
        }
    }
}


impl<'a, F> GBM<'a, F> {
    /// Returns the maximum iteration
    /// of the `GBM` to find a combined hypothesis
    /// that has error at most `tolerance`.
    /// Default max loop is `100`.
    pub fn max_loop(&self) -> usize {
        let n_sample = self.data.shape().0 as f64;

        (n_sample.ln() / self.tolerance.powi(2)) as usize
    }


    /// Set the tolerance parameter.
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }


    /// Set the Loss Type.
    pub fn loss(mut self, loss_type: GBMLoss) -> Self {
        self.loss = loss_type;
        self
    }
}


impl<F> Booster<F> for GBM<'_, F>
    where F: Regressor + Clone,
{
    fn preprocess<W>(
        &mut self,
        _weak_learner: &W,
    )
        where W: WeakLearner<Hypothesis = F>
    {
        // Initialize parameters
        let n_sample = self.data.shape().0;

        self.weights = Vec::with_capacity(self.max_iter);
        self.classifiers = Vec::with_capacity(self.max_iter);

        self.residuals = self.target.f64()
            .unwrap()
            .into_iter()
            .map(Option::unwrap)
            .collect::<Vec<f64>>();

        self.ones = vec![1.0; n_sample];


        self.terminated = self.max_iter;
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


        // Get a new hypothesis
        let target = Series::new(&"target", &self.residuals[..]);
        let h = weak_learner.produce(self.data, &target, &self.ones[..]);

        let predictions = h.predict_all(self.data);
        let coef = self.loss.best_coefficient(
            &self.residuals[..], &predictions[..]
        );

        // If the best coefficient is zero,
        // the newly-attained hypothesis `h` do nothing.
        // Thus, we can terminate the boosting at this point.
        if coef == 0.0 {
            self.terminated = iteration;
            return State::Terminate;
        }

        // Update the residual vector
        self.residuals.iter_mut()
            .zip(predictions)
            .for_each(|(r, p)| {
                *r -= coef * p;
            });


        self.weights.push(coef);
        self.classifiers.push(h);

        State::Continue
    }


    fn postprocess<W>(
        &mut self,
        _weak_learner: &W,
    ) -> CombinedHypothesis<F>
        where W: WeakLearner<Hypothesis = F>
    {
        let f = self.weights.iter()
            .copied()
            .zip(self.classifiers.iter().cloned())
            .filter(|(w, _)| *w != 0.0)
            .collect::<Vec<_>>();
        CombinedHypothesis::from(f)
    }
}

