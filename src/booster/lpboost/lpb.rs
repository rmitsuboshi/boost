//! This file defines `LPBoost` based on the paper
//! ``Boosting algorithms for Maximizing the Soft Margin''
//! by Warmuth et al.
//! 
use polars::prelude::*;
// use rayon::prelude::*;

use super::lp_model::LPModel;

use crate::{
    Booster,
    BaseLearner,
    State,

    Classifier,
    CombinedClassifier,
};


use std::cell::RefCell;



/// LPBoost struct.
/// See [this paper](https://proceedings.neurips.cc/paper/2007/file/cfbce4c1d7c425baf21d6b6f2babe6be-Paper.pdf).
pub struct LPBoost<C> {
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


    classifiers: Vec<C>,


    terminated: usize,
}


impl<C> LPBoost<C>
    where C: Classifier
{
    /// Initialize the `LPBoost`.
    pub fn init(data: &DataFrame, _target: &Series) -> Self {
        let (n_sample, _) = data.shape();
        assert!(n_sample != 0);


        let uni = 1.0 / n_sample as f64;
        LPBoost {
            dist:      vec![uni; n_sample],
            gamma_hat: 1.0,
            tolerance: uni,
            n_sample,
            nu:        1.0,
            lp_model: None,

            classifiers: Vec::new(),


            terminated: 0_usize,
        }
    }


    /// This method updates the capping parameter.
    /// This parameter must be in `[1, n_sample]`.
    pub fn nu(mut self, nu: f64) -> Self {
        assert!(1.0 <= nu && nu <= self.n_sample as f64);
        self.nu = nu;

        self
    }


    fn init_solver(&mut self) {
        let upper_bound = 1.0 / self.nu;

        assert!((0.0..=1.0).contains(&upper_bound));

        let lp_model = RefCell::new(LPModel::init(self.n_sample, upper_bound));

        self.lp_model = Some(lp_model);
    }


    /// Set the tolerance parameter.
    /// Default is `1.0 / sample_size`/
    #[inline(always)]
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }


    /// Returns the terminated iteration.
    /// This method returns `0` before the boosting step.
    #[inline(always)]
    pub fn terminated(&self) -> usize {
        self.terminated
    }


    /// This method updates `self.dist` and `self.gamma_hat`
    /// by solving a linear program
    /// over the hypotheses obtained in past steps.
    #[inline(always)]
    fn update_distribution_mut(
        &self,
        data: &DataFrame,
        target: &Series,
        h: &C
    ) -> f64
    {
        self.lp_model.as_ref()
            .unwrap()
            .borrow_mut()
            .update(data, target, h)
    }
}


impl<C> Booster<C> for LPBoost<C>
    where C: Classifier + Clone,
{
    fn preprocess<B>(
        &mut self,
        _base_learner: &B,
        data: &DataFrame,
        _target: &Series,
    )
        where B: BaseLearner<Clf = C>
    {
        let n_sample = data.shape().0;
        let uni = 1.0_f64 / n_sample as f64;

        self.init_solver();

        self.n_sample = n_sample;
        self.dist = vec![uni; n_sample];
        self.gamma_hat = 1.0;
        self.classifiers = Vec::new();
        self.terminated = usize::MAX;
    }


    fn boost<B>(
        &mut self,
        base_learner: &B,
        data: &DataFrame,
        target: &Series,
        _iteration: usize,
    ) -> State
        where B: BaseLearner<Clf = C>,
    {
        let h = base_learner.produce(data, target, &self.dist);

        // Each element in `margins` is the product of
        // the predicted vector and the correct vector

        let ghat = target.i64()
            .expect("The target class is not a dtype of i64")
            .into_iter()
            .enumerate()
            .map(|(i, y)| y.unwrap() as f64 * h.confidence(data, i))
            .zip(self.dist.iter())
            .map(|(yh, &d)| d * yh)
            .sum::<f64>();

        self.gamma_hat = ghat.min(self.gamma_hat);


        let gamma_star = self.update_distribution_mut(
            data, target, &h
        );


        self.classifiers.push(h);

        if gamma_star >= self.gamma_hat - self.tolerance {
            self.terminated = self.classifiers.len();
            return State::Terminate;
        }

        // Update the distribution over the training examples.
        self.dist = self.lp_model.as_ref()
            .unwrap()
            .borrow()
            .distribution();

        State::Continue
    }


    fn postprocess<B>(
        &mut self,
        _base_learner: &B,
        _data: &DataFrame,
        _target: &Series,
    ) -> CombinedClassifier<C>
        where B: BaseLearner<Clf = C>
    {
        let clfs = self.lp_model.as_ref()
            .unwrap()
            .borrow()
            .weight()
            .zip(self.classifiers.clone())
            .filter(|(w, _)| *w != 0.0)
            .collect::<Vec<(f64, C)>>();


        CombinedClassifier::from(clfs)
    }
}


// impl<C> Booster<C> for LPBoost
//     where C: Classifier,
// {
//     fn run<B>(
//         &mut self,
//         base_learner: &B,
//         data: &DataFrame,
//         target: &Series,
//     ) -> CombinedClassifier<C>
//         where B: BaseLearner<Clf = C>,
//     {
//         self.init_solver();
// 
//         let mut classifiers = Vec::new();
// 
//         self.terminated = usize::MAX;
// 
//         // Since the LPBoost does not have non-trivial iteration,
//         // we run this until the stopping criterion is satisfied.
//         loop {
//             let h = base_learner.produce(data, target, &self.dist);
// 
//             // Each element in `margins` is the product of
//             // the predicted vector and the correct vector
// 
//             let ghat = target.i64()
//                 .expect("The target class is not a dtype of i64")
//                 .into_iter()
//                 .enumerate()
//                 .map(|(i, y)| y.unwrap() as f64 * h.confidence(data, i))
//                 .zip(self.dist.iter())
//                 .map(|(yh, &d)| d * yh)
//                 .sum::<f64>();
// 
//             self.gamma_hat = ghat.min(self.gamma_hat);
// 
// 
//             let gamma_star = self.update_distribution_mut(
//                 data, target, &h
//             );
// 
// 
//             classifiers.push(h);
// 
//             if gamma_star >= self.gamma_hat - self.tolerance {
//                 println!("Break loop at: {t}", t = classifiers.len());
//                 self.terminated = classifiers.len();
//                 break;
//             }
// 
//             // Update the distribution over the training examples.
//             self.dist = self.lp_model.as_ref()
//                 .unwrap()
//                 .borrow()
//                 .distribution();
//         }
// 
// 
//         let clfs = self.lp_model.as_ref()
//             .unwrap()
//             .borrow()
//             .weight()
//             .zip(classifiers)
//             .filter(|(w, _)| *w != 0.0)
//             .collect::<Vec<(f64, C)>>();
// 
// 
//         CombinedClassifier::from(clfs)
//     }
// }