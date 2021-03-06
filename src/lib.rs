//! See [`optimize`] for documentation on the Levenberg-Marquardt optimization algorithm.

#![no_std]

use nalgebra::{
    allocator::Allocator,
    constraint::{DimEq, ShapeConstraint},
    dimension::{DimMin, DimMinimum},
    storage::{ContiguousStorageMut, Storage},
    DefaultAllocator, Dim, DimName, Matrix, MatrixMN, RealField, Vector,
};

use num_traits::FromPrimitive;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Config<N> {
    pub max_iterations: usize,
    pub consecutive_divergence_limit: usize,
    pub initial_lambda: N,
    pub lambda_convege: N,
    pub lambda_diverge: N,
    pub threshold: N,
}

impl<N> Default for Config<N>
where
    N: FromPrimitive,
{
    fn default() -> Self {
        Self {
            max_iterations: 1000,
            consecutive_divergence_limit: 5,
            initial_lambda: N::from_f32(50.0)
                .expect("leverberg-marquardt vector and matrix type cant store 50.0"),
            lambda_convege: N::from_f32(0.8)
                .expect("leverberg-marquardt vector and matrix type cant store 0.8"),
            lambda_diverge: N::from_f32(2.0)
                .expect("leverberg-marquardt vector and matrix type cant store 2.0"),
            threshold: N::from_f32(0.0)
                .expect("leverberg-marquardt vector and matrix type cant store 0.0"),
        }
    }
}

/// Note that the differentials and state vector are represented with column vectors.
/// This is atypical from the normal way it is done in mathematics. This is done because
/// nalgebra is column-major. A nalgebra `Vector` is a column vector.
///
/// Make sure that you create your Jacobian such that it is several fixed length
/// column vectors rather than several row vectors as per normal. If you have already
/// computed it with row vectors, then you can take the transpose.
///
/// It is recommended to make the number of columns dynamic unless you have a small fixed
/// number of data-points.
///
/// `max_iterations` limits the number of times the initial guess will be updated.
///
/// `consecutive_divergence_limit` limits the number of times that lambda can diverge
/// consecutively from Gauss-Newton due to a failed improvement. Once the
/// solution is as good as possible, it will begin regressing to gradient descent. This
/// limit prevents it from wasting the remaining cycles of the algorithm.
///
/// `initial_lambda` defines the initial lambda value. As lambda grows higher,
/// Levenberg-Marquardt approaches gradient descent, which is better at converging to a distant
/// minima. As lambda grows lower, Levenberg-Marquardt approaches Gauss-Newton, which allows faster
/// convergence closer to the minima. A lambda of `0.0` would imply that it is purely based on
/// Gauss-Newton approximation. Please do not set lambda to exactly `0.0` or the `lambda_scale` will be unable to
/// increase lambda since it does so through multiplication.
///
/// `lambda_converge` must be set to a value below `1.0`. On each iteration of Levenberg-Marquardt,
/// the lambda is used as-is and multiplied by `lambda_converge`. If the original lambda or the
/// new lambda is better, that lambda becomes the new lambda. If neither are better than the
/// previous sum-of-squares, then lambda is multiplied by `lambda_diverge`.
///
/// `lambda_diverge` must be set to a value above `1.0` and highly recommended to set it **above**
/// `lambda_converge^-1` (it will re-test an already-used lambda otherwise). On each iteration,
/// if the sum-of-squares regresses, then lambda is multiplied by `lambda_diverge` to move closer
/// to gradient descent in hopes that it will cause it to converge.
///
/// `threshold` is the point at which the average-of-squares is low enough that the algorithm can
/// terminate. This exists so that the algorithm can short-circuit and exit early if the
/// solution was easy to find. Set this to `0.0` if you want it to continue for all `max_iterations`.
/// You might do that if you always have a fixed amount of time per optimization, such as when
/// processing live video frames.
///
/// `init` is the initial parameter guess. Make sure to set `init` close to the actual solution.
/// It is recommended to use a sample consensus algorithm to get a close initial approximation.
///
/// `normalize` allows the parameter guess to be normalized on each iteration. It can be pushed into
/// a slightly incorrect state on each iteration and this can be used to correct it. This might be something
/// like an angle which exceeds 2 * pi. It might technically be correct, but you want to wrap it back around.
/// This is also useful when a normal vector or unit quaternion is involved since those need to be kept
/// normalized throughout the optimization procedure.
///
/// `residuals` must return the difference between the expected value and the output of the
/// function being optimized. This is returned as a matrix where the number of residuals (rows)
/// that are present in each column must correspond to the number of columns in each
/// Jacobian returned by `jacobians`.
///
/// `jacobians` is a function that takes in the current guess and produces all the Jacobian
/// matrices of the negative residuals in respect to the parameter. Each row should correspond to
/// a dimension of the parameter vector and each column should correspond to a row in the residual matrix.
/// You can pass in the Jacobian of as many residuals as you would like on each iteration,
/// so long as the residuals returned by `residuals` has the same number of residuals per column.
/// Only the Jacobian and the residuals are required to perform Levenberg-Marquardt optimization.
/// You may need to caputure your observances in the closure to compute the Jacobian, but
/// they are not arguments since they are constants to Levenberg-Marquardt.
///
/// `M` is the model that is being optimized.
///
/// `N` is the type parameter of the data type that is stored in the matrix (`f32`).
///
/// `P` is the number of parameter variables being optimized.
///
/// `S` is the number of samples used in optimization.
///
/// `J` is the number of rows per sample returned and the number of columns in the Jacobian.
///
/// `PS` is the nalgebra storage used for the parameter vector.
///
/// `RS` is the nalgebra storage used for the residual matrix.
///
/// `JS` is the nalgebra storage used for the Jacobian matrix.
///
/// `IJ` is the iterator over the Jacobian matrices of each sample.
pub fn optimize<M, N, P, S, J, PS, RS, JS, IJ>(
    config: Config<N>,
    init: M,
    apply_delta: impl Fn(&M, Vector<N, P, PS>) -> M,
    residuals: impl Fn(&M) -> Matrix<N, J, S, RS>,
    jacobians: impl Fn(&M) -> IJ,
) -> M
where
    N: RealField + FromPrimitive,
    P: DimMin<P> + DimName,
    S: Dim,
    J: DimName,
    PS: ContiguousStorageMut<N, P> + Clone,
    RS: Storage<N, J, S>,
    JS: Storage<N, P, J>,
    IJ: Iterator<Item = Matrix<N, P, J, JS>>,
    DefaultAllocator: Allocator<N, J, P>,
    DefaultAllocator: Allocator<N, P, P>,
    DefaultAllocator: Allocator<N, P, Buffer = PS>,
    ShapeConstraint: DimEq<DimMinimum<P, P>, P>,
{
    let mut lambda = config.initial_lambda;
    let mut guess = init;
    let mut res = residuals(&guess);
    let mut sum_of_squares = res.norm_squared();
    let mut consecutive_divergences = 0;
    let total = N::from_usize(res.len())
        .expect("there were more items in the vector than could be represented by the type");

    for _ in 0..config.max_iterations {
        // Next step lambda.
        let smaller_lambda = lambda * config.lambda_convege;

        // Iterate through all the Jacobians to extract the approximate Hessian and the gradients.
        let (hessian, gradients) = jacobians(&guess).zip(res.column_iter()).fold(
            (nalgebra::zero(), nalgebra::zero()),
            |(hessian, gradients): (MatrixMN<N, P, P>, Vector<N, P, PS>), (jacobian, res)| {
                (
                    hessian + &jacobian * jacobian.transpose(),
                    gradients + &jacobian * res,
                )
            },
        );

        // Get a tuple of the lambda, guess, residual, and sum-of-squares.
        // Returns an option because it may not be possible to solve the inverse.
        let lam_ges_res_sum = |lam| {
            // Compute JJᵀ + λ*diag(JJᵀ).
            let mut hessian_lambda_diag = hessian.clone();
            let new_diag = hessian_lambda_diag.map_diagonal(|n| n * (lam + N::one()));
            hessian_lambda_diag.set_diagonal(&new_diag);

            // Invert JᵀJ + λ*diag(JᵀJ) and solve for delta.
            let delta = hessian_lambda_diag
                .try_inverse()
                .map(|inv_jjl| inv_jjl * &gradients);
            // Compute the new guess, residuals, and sum-of-squares.
            let vars = delta.map(|delta| {
                let ges = apply_delta(&guess, delta);
                let res = residuals(&ges);
                let sum = res.norm_squared();
                (lam, ges, res, sum)
            });
            // If the sum-of-squares is infinite or NaN it shouldn't be allowed through.
            vars.filter(|vars| vars.3.is_finite())
        };

        // Select the vars that minimize the sum-of-squares the most.
        let new_vars = match (lam_ges_res_sum(smaller_lambda), lam_ges_res_sum(lambda)) {
            (Some(s_vars), Some(o_vars)) => Some(if s_vars.3 < o_vars.3 { s_vars } else { o_vars }),
            (Some(vars), None) | (None, Some(vars)) => Some(vars),
            (None, None) => None,
        };

        if let Some((n_lam, n_ges, n_res, n_sum)) = new_vars {
            // We didn't see a decrease in the new state.
            if n_sum > sum_of_squares {
                // Increase lambda twice and go to the next iteration.
                // Increase twice so that the new two tested lambdas are different than current.
                lambda *= config.lambda_diverge;
                consecutive_divergences += 1;
            } else {
                // There was a decrease, so update everything.
                lambda = n_lam;
                guess = n_ges;
                res = n_res;
                sum_of_squares = n_sum;
                consecutive_divergences = 0;
            }
        } else {
            // We were unable to take the inverse, so increase lambda in hopes that it may
            // cause the matrix to become invertible.
            lambda *= config.lambda_diverge;
            consecutive_divergences += 1;
        }

        // Terminate early if we hit the consecutive divergence limit.
        if consecutive_divergences == config.consecutive_divergence_limit {
            break;
        }

        // We can terminate early if the sum of squares is below the threshold.
        if sum_of_squares < config.threshold * total {
            break;
        }
    }

    guess
}
