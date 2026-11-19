//! Estimators used by the Paper 2 programs.
//!
//! * `fe_ols`: least squares with any number of absorbed fixed effects
//!   (removed by alternating projections) and standard errors clustered
//!   on one dimension.
//! * `clogit_fit`: conditional logit, fitted by Newton's method, with
//!   sparse regressors and observation weights. The weights are used for
//!   bootstrap draws (a group drawn twice gets weight 2) and for
//!   reweighting checks.
//! * Small helpers: linear solves, normal tail probability, percentiles,
//!   the Holm correction, weighted medians.
//!
//! No function here reads data or holds data values.

use std::collections::HashMap;

// ---------------------------------------------------------------- algebra

/// Cholesky factor of a symmetric positive definite matrix, or `None`.
fn cholesky(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut s = a[i][j];
            for k in 0..j {
                s -= l[i][k] * l[j][k];
            }
            if i == j {
                if !(s > 0.0) || !s.is_finite() {
                    return None;
                }
                l[i][i] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }
    Some(l)
}

fn cholesky_solve_with(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let n = l.len();
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= l[i][k] * y[k];
        }
        y[i] = s / l[i][i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= l[k][i] * x[k];
        }
        x[i] = s / l[i][i];
    }
    x
}

/// Solve `a x = b` for symmetric positive definite `a`.
pub fn solve_spd(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    cholesky(a).map(|l| cholesky_solve_with(&l, b))
}

/// Inverse of a symmetric positive definite matrix.
pub fn inverse_spd(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let l = cholesky(a)?;
    let n = a.len();
    let mut inv = vec![vec![0.0; n]; n];
    for j in 0..n {
        let mut e = vec![0.0; n];
        e[j] = 1.0;
        let col = cholesky_solve_with(&l, &e);
        for i in 0..n {
            inv[i][j] = col[i];
        }
    }
    Some(inv)
}

/// `a b a` for square matrices, used for sandwich variances.
fn sandwich(bread: &[Vec<f64>], meat: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = bread.len();
    let mut tmp = vec![vec![0.0; n]; n];
    for i in 0..n {
        for k in 0..n {
            if bread[i][k] == 0.0 {
                continue;
            }
            for j in 0..n {
                tmp[i][j] += bread[i][k] * meat[k][j];
            }
        }
    }
    let mut out = vec![vec![0.0; n]; n];
    for i in 0..n {
        for k in 0..n {
            if tmp[i][k] == 0.0 {
                continue;
            }
            for j in 0..n {
                out[i][j] += tmp[i][k] * bread[k][j];
            }
        }
    }
    out
}

// ------------------------------------------------------------ small stats

/// Two-sided p-value of a z statistic under the standard normal.
pub fn two_sided_p(z: f64) -> f64 {
    erfc(z.abs() / std::f64::consts::SQRT_2)
}

/// Complementary error function (Numerical Recipes `erfcc`, relative
/// error below 1.2e-7).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 {
        ans
    } else {
        2.0 - ans
    }
}

/// Percentile of a sorted sample, by the nearest rank on (n - 1) p.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

/// Standard deviation of a sample.
pub fn sd(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    if n < 2.0 {
        return f64::NAN;
    }
    let mean = values.iter().sum::<f64>() / n;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
}

/// Median with weights: the smallest value at which the cumulative weight
/// reaches half of the total.
pub fn weighted_median(values: &mut [(f64, f64)]) -> f64 {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = values.iter().map(|v| v.1).sum();
    let mut cumulative = 0.0;
    for &(value, weight) in values.iter() {
        cumulative += weight;
        if cumulative >= 0.5 * total {
            return value;
        }
    }
    f64::NAN
}

/// Holm adjusted p-values, in the order given.
pub fn holm(p: &[f64]) -> Vec<f64> {
    let m = p.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| p[a].total_cmp(&p[b]));
    let mut adjusted = vec![0.0; m];
    let mut running: f64 = 0.0;
    for (rank, &i) in order.iter().enumerate() {
        let value = ((m - rank) as f64 * p[i]).min(1.0);
        running = running.max(value);
        adjusted[i] = running;
    }
    adjusted
}

// -------------------------------------------------- fixed-effects least squares

/// Result of `fe_ols`.
#[derive(Debug, Clone)]
pub struct FeFit {
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub n_obs: usize,
    pub n_clusters: usize,
    /// Share of the variance of the demeaned outcome explained by the regressors.
    pub r2_within: f64,
}

impl FeFit {
    pub fn z(&self, j: usize) -> f64 {
        self.coef[j] / self.se[j]
    }
    pub fn p(&self, j: usize) -> f64 {
        two_sided_p(self.z(j))
    }
}

/// Remove the weighted means of every fixed effect from `v`, by
/// alternating projections until the largest change is below `tol`.
fn demean(v: &mut [f64], fes: &[Vec<usize>], levels: &[usize], w: &[f64], tol: f64) {
    if fes.is_empty() {
        return;
    }
    let mut sums: Vec<Vec<f64>> = levels.iter().map(|&l| vec![0.0; l]).collect();
    let mut weights: Vec<Vec<f64>> = levels.iter().map(|&l| vec![0.0; l]).collect();
    for (f, fe) in fes.iter().enumerate() {
        for (i, &g) in fe.iter().enumerate() {
            weights[f][g] += w[i];
        }
    }
    for _ in 0..100_000 {
        let mut largest: f64 = 0.0;
        for (f, fe) in fes.iter().enumerate() {
            sums[f].iter_mut().for_each(|s| *s = 0.0);
            for (i, &g) in fe.iter().enumerate() {
                sums[f][g] += w[i] * v[i];
            }
            for (i, &g) in fe.iter().enumerate() {
                if weights[f][g] > 0.0 {
                    let mean = sums[f][g] / weights[f][g];
                    v[i] -= mean;
                    largest = largest.max(mean.abs());
                }
            }
        }
        if largest < tol || fes.len() == 1 {
            return;
        }
    }
}

/// Least squares of `y` on the columns of `x`, with the fixed effects in
/// `fes` absorbed. Each entry of `fes` gives, for every observation, the
/// level (0-based) of one fixed effect. Standard errors are clustered on
/// `clusters`, with the usual small-sample factor
/// G / (G - 1) × (N - 1) / (N - K), where K counts the regressors and the
/// absorbed levels. Weights are frequency weights.
pub fn fe_ols(
    y: &[f64],
    x: &[Vec<f64>],
    fes: &[Vec<usize>],
    w: Option<&[f64]>,
    clusters: &[usize],
) -> Option<FeFit> {
    let n = y.len();
    let ones = vec![1.0; n];
    let w = w.unwrap_or(&ones);
    let levels: Vec<usize> =
        fes.iter().map(|fe| fe.iter().copied().max().map_or(0, |m| m + 1)).collect();
    let tol = 1e-11;
    let mut yd = y.to_vec();
    demean(&mut yd, fes, &levels, w, tol);
    let xd: Vec<Vec<f64>> = x
        .iter()
        .map(|col| {
            let mut c = col.clone();
            demean(&mut c, fes, &levels, w, tol);
            c
        })
        .collect();
    let k = x.len();
    let mut xtx = vec![vec![0.0; k]; k];
    let mut xty = vec![0.0; k];
    for i in 0..n {
        if w[i] == 0.0 {
            continue;
        }
        for a in 0..k {
            xty[a] += w[i] * xd[a][i] * yd[i];
            for b in 0..=a {
                xtx[a][b] += w[i] * xd[a][i] * xd[b][i];
            }
        }
    }
    for a in 0..k {
        for b in 0..a {
            xtx[b][a] = xtx[a][b];
        }
    }
    let coef = solve_spd(&xtx, &xty)?;
    let bread = inverse_spd(&xtx)?;
    let resid: Vec<f64> =
        (0..n).map(|i| yd[i] - (0..k).map(|a| coef[a] * xd[a][i]).sum::<f64>()).collect();
    let mut scores: HashMap<usize, Vec<f64>> = HashMap::new();
    let mut n_obs = 0.0;
    for i in 0..n {
        if w[i] == 0.0 {
            continue;
        }
        n_obs += w[i];
        let s = scores.entry(clusters[i]).or_insert_with(|| vec![0.0; k]);
        for a in 0..k {
            s[a] += w[i] * xd[a][i] * resid[i];
        }
    }
    let g = scores.len() as f64;
    let mut meat = vec![vec![0.0; k]; k];
    for s in scores.values() {
        for a in 0..k {
            for b in 0..k {
                meat[a][b] += s[a] * s[b];
            }
        }
    }
    let absorbed: f64 = levels.iter().map(|&l| l as f64).sum::<f64>()
        - (levels.len().saturating_sub(1)) as f64;
    let k_total = k as f64 + absorbed;
    let factor = if g > 1.0 && n_obs > k_total {
        g / (g - 1.0) * (n_obs - 1.0) / (n_obs - k_total)
    } else {
        f64::NAN
    };
    let v = sandwich(&bread, &meat);
    let se = (0..k).map(|a| (factor * v[a][a]).sqrt()).collect();
    let tss: f64 = (0..n).map(|i| w[i] * yd[i] * yd[i]).sum();
    let rss: f64 = (0..n).map(|i| w[i] * resid[i] * resid[i]).sum();
    Some(FeFit {
        coef,
        se,
        n_obs: n_obs.round() as usize,
        n_clusters: scores.len(),
        r2_within: 1.0 - rss / tss,
    })
}

// ------------------------------------------------------ conditional logit

/// One alternative: sparse regressors as (index, value).
#[derive(Debug, Clone, Default)]
pub struct Alt {
    pub x: Vec<(usize, f64)>,
}

/// One choice among alternatives.
#[derive(Debug, Clone)]
pub struct Choice {
    pub alts: Vec<Alt>,
    pub chosen: usize,
    /// Cluster for the sandwich variance and for the bootstrap.
    pub cluster: usize,
}

#[derive(Debug, Clone)]
pub struct ClogitFit {
    pub beta: Vec<f64>,
    pub loglik: f64,
    /// Negative Hessian of the weighted log-likelihood at `beta`.
    pub info: Vec<Vec<f64>>,
    pub iterations: usize,
    pub converged: bool,
}

fn scores_of(alt: &Alt, beta: &[f64]) -> f64 {
    alt.x.iter().map(|&(j, v)| beta[j] * v).sum()
}

/// Log probability of each alternative of a choice.
fn log_probs(c: &Choice, beta: &[f64]) -> Vec<f64> {
    let s: Vec<f64> = c.alts.iter().map(|a| scores_of(a, beta)).collect();
    let max = s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let lse = max + s.iter().map(|v| (v - max).exp()).sum::<f64>().ln();
    s.iter().map(|v| v - lse).collect()
}

/// Log-likelihood contribution of each choice (unweighted).
pub fn clogit_loglik_each(choices: &[Choice], beta: &[f64]) -> Vec<f64> {
    choices.iter().map(|c| log_probs(c, beta)[c.chosen]).collect()
}

/// Probabilities of the alternatives of one choice.
pub fn clogit_probs(c: &Choice, beta: &[f64]) -> Vec<f64> {
    log_probs(c, beta).into_iter().map(f64::exp).collect()
}

/// Weighted log-likelihood, gradient and negative Hessian.
fn clogit_derivatives(
    choices: &[Choice],
    w: &[f64],
    beta: &[f64],
    k: usize,
) -> (f64, Vec<f64>, Vec<Vec<f64>>) {
    let mut ll = 0.0;
    let mut grad = vec![0.0; k];
    let mut info = vec![vec![0.0; k]; k];
    let mut xbar = vec![0.0; k];
    let mut touched: Vec<usize> = Vec::new();
    for (c, &wc) in choices.iter().zip(w) {
        if wc == 0.0 {
            continue;
        }
        let lp = log_probs(c, beta);
        ll += wc * lp[c.chosen];
        touched.clear();
        for (a, alt) in c.alts.iter().enumerate() {
            let p = lp[a].exp();
            for &(j, v) in &alt.x {
                if xbar[j] == 0.0 && !touched.contains(&j) {
                    touched.push(j);
                }
                xbar[j] += p * v;
            }
            for &(i, vi) in &alt.x {
                for &(j, vj) in &alt.x {
                    info[i][j] += wc * p * vi * vj;
                }
            }
        }
        for &(j, v) in &c.alts[c.chosen].x {
            grad[j] += wc * v;
        }
        for &i in &touched {
            grad[i] -= wc * xbar[i];
            for &j in &touched {
                info[i][j] -= wc * xbar[i] * xbar[j];
            }
        }
        for &i in &touched {
            xbar[i] = 0.0;
        }
    }
    (ll, grad, info)
}

/// Fit a conditional logit with `k` parameters by Newton's method with
/// step halving. `w` gives a weight to each choice.
pub fn clogit_fit(choices: &[Choice], w: &[f64], k: usize, start: &[f64]) -> ClogitFit {
    let mut beta = start.to_vec();
    let (mut ll, mut grad, mut info) = clogit_derivatives(choices, w, &beta, k);
    let mut converged = false;
    let mut iterations = 0;
    for it in 0..200 {
        iterations = it + 1;
        // A tiny ridge keeps the solve stable if a parameter has no variation.
        let mut ridge = 1e-10;
        let step = loop {
            let mut m = info.clone();
            for (i, row) in m.iter_mut().enumerate() {
                row[i] += ridge;
            }
            if let Some(s) = solve_spd(&m, &grad) {
                break s;
            }
            ridge *= 100.0;
            if ridge > 1e3 {
                break grad.iter().map(|g| 1e-3 * g).collect();
            }
        };
        let mut t = 1.0;
        let mut accepted = false;
        for _ in 0..40 {
            let trial: Vec<f64> = beta.iter().zip(&step).map(|(b, s)| b + t * s).collect();
            let ll_trial: f64 = clogit_loglik_each(choices, &trial)
                .iter()
                .zip(w)
                .map(|(l, wi)| l * wi)
                .sum();
            if ll_trial >= ll - 1e-12 {
                beta = trial;
                accepted = true;
                break;
            }
            t *= 0.5;
        }
        let max_step = step.iter().map(|s| (t * s).abs()).fold(0.0, f64::max);
        let (ll_new, grad_new, info_new) = clogit_derivatives(choices, w, &beta, k);
        let gain = ll_new - ll;
        ll = ll_new;
        grad = grad_new;
        info = info_new;
        let max_grad = grad.iter().map(|g| g.abs()).fold(0.0, f64::max);
        if max_grad < 1e-6 || max_step < 1e-9 || (accepted && gain.abs() < 1e-11) {
            converged = max_grad < 1e-4;
            break;
        }
        if !accepted {
            converged = max_grad < 1e-4;
            break;
        }
    }
    ClogitFit { beta, loglik: ll, info, iterations, converged }
}

/// Standard errors from the inverse information (model based) and from a
/// sandwich clustered on `Choice::cluster`. `w` are the same choice
/// weights as in the fit; choices with weight 0 drop out. Parameters with
/// no information in the weighted sample (for example a seat level that
/// occurs only in the hold-out) get NaN, and the others are computed
/// without them.
pub fn clogit_se(choices: &[Choice], w: &[f64], fit: &ClogitFit) -> (Vec<f64>, Vec<f64>) {
    let k_all = fit.beta.len();
    let active: Vec<usize> = (0..k_all).filter(|&i| fit.info[i][i] > 1e-9).collect();
    let pos: HashMap<usize, usize> = active.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    let k = active.len();
    let sub: Vec<Vec<f64>> =
        active.iter().map(|&i| active.iter().map(|&j| fit.info[i][j]).collect()).collect();
    let mut model_all = vec![f64::NAN; k_all];
    let mut robust_all = vec![f64::NAN; k_all];
    let Some(bread) = inverse_spd(&sub) else {
        return (model_all, robust_all);
    };
    let mut by_cluster: HashMap<usize, Vec<f64>> = HashMap::new();
    for (c, &wc) in choices.iter().zip(w) {
        if wc == 0.0 {
            continue;
        }
        let p = clogit_probs(c, &fit.beta);
        let s = by_cluster.entry(c.cluster).or_insert_with(|| vec![0.0; k]);
        for &(j, v) in &c.alts[c.chosen].x {
            if let Some(&q) = pos.get(&j) {
                s[q] += wc * v;
            }
        }
        for (a, alt) in c.alts.iter().enumerate() {
            for &(j, v) in &alt.x {
                if let Some(&q) = pos.get(&j) {
                    s[q] -= wc * p[a] * v;
                }
            }
        }
    }
    let g = by_cluster.len() as f64;
    let mut meat = vec![vec![0.0; k]; k];
    for s in by_cluster.values() {
        for a in 0..k {
            if s[a] == 0.0 {
                continue;
            }
            for b in 0..k {
                meat[a][b] += s[a] * s[b];
            }
        }
    }
    let v = sandwich(&bread, &meat);
    let factor = if g > 1.0 { g / (g - 1.0) } else { f64::NAN };
    for (p, &i) in active.iter().enumerate() {
        model_all[i] = bread[p][p].sqrt();
        robust_all[i] = (factor * v[p][p]).sqrt();
    }
    (model_all, robust_all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn holm_matches_hand_calculation() {
        let adj = holm(&[0.01, 0.04]);
        assert!((adj[0] - 0.02).abs() < 1e-12);
        assert!((adj[1] - 0.04).abs() < 1e-12);
        let adj = holm(&[0.03, 0.02]);
        assert!((adj[0] - 0.04).abs() < 1e-12 && (adj[1] - 0.04).abs() < 1e-12);
    }

    #[test]
    fn normal_tail_is_right() {
        assert!((two_sided_p(1.959_964) - 0.05).abs() < 1e-6);
        assert!((two_sided_p(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fe_ols_recovers_a_slope_with_two_way_effects() {
        let mut rng = crate::seeded_rng(1);
        let (n, ga, gb) = (4000, 50, 30);
        let a: Vec<usize> = (0..n).map(|_| rng.gen_range(0..ga)).collect();
        let b: Vec<usize> = (0..n).map(|_| rng.gen_range(0..gb)).collect();
        let x: Vec<f64> = (0..n).map(|i| rng.gen::<f64>() + 0.1 * a[i] as f64).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| 2.0 * x[i] + a[i] as f64 * 0.3 - b[i] as f64 * 0.7 + 0.1 * rng.gen::<f64>())
            .collect();
        let fit = fe_ols(&y, &[x], &[a.clone(), b], None, &a).unwrap();
        assert!((fit.coef[0] - 2.0).abs() < 0.02, "{:?}", fit.coef);
        assert!(fit.se[0] > 0.0 && fit.se[0] < 0.05);
    }

    #[test]
    fn clogit_recovers_its_parameters() {
        let mut rng = crate::seeded_rng(2);
        let truth = [0.8, -1.2];
        let mut choices = Vec::new();
        for i in 0..6000 {
            let alts: Vec<Alt> = (0..4)
                .map(|_| Alt { x: vec![(0, rng.gen::<f64>()), (1, rng.gen_range(0..2) as f64)] })
                .collect();
            let p = clogit_probs(&Choice { alts: alts.clone(), chosen: 0, cluster: i }, &truth);
            let u: f64 = rng.gen();
            let mut acc = 0.0;
            let mut chosen = 3;
            for (a, pa) in p.iter().enumerate() {
                acc += pa;
                if u < acc {
                    chosen = a;
                    break;
                }
            }
            choices.push(Choice { alts, chosen, cluster: i });
        }
        let w = vec![1.0; choices.len()];
        let fit = clogit_fit(&choices, &w, 2, &[0.0, 0.0]);
        assert!(fit.converged);
        assert!((fit.beta[0] - 0.8).abs() < 0.15, "{:?}", fit.beta);
        assert!((fit.beta[1] + 1.2).abs() < 0.1, "{:?}", fit.beta);
        let (model, robust) = clogit_se(&choices, &w, &fit);
        assert!((model[1] / robust[1] - 1.0).abs() < 0.2);
        let w2 = vec![2.0; choices.len()];
        let fit2 = clogit_fit(&choices, &w2, 2, &[0.0, 0.0]);
        let (_, robust2) = clogit_se(&choices, &w2, &fit2);
        let mut twice = choices.clone();
        twice.extend(choices.iter().cloned().map(|mut c| {
            c.cluster += choices.len();
            c
        }));
        let w1 = vec![1.0; twice.len()];
        let fit1 = clogit_fit(&twice, &w1, 2, &[0.0, 0.0]);
        let (_, robust1) = clogit_se(&twice, &w1, &fit1);
        // Doubling every weight leaves the clusters as they are, so the
        // clustered se must not change. Copying the data into new clusters
        // shrinks it by sqrt(2).
        assert!((robust2[1] / robust[1] - 1.0).abs() < 1e-6, "{robust2:?} {robust:?}");
        assert!((robust1[1] * 2f64.sqrt() / robust[1] - 1.0).abs() < 0.05, "{robust1:?} {robust:?}");
    }

    #[test]
    fn a_parameter_without_information_does_not_spoil_the_others() {
        let mut rng = crate::seeded_rng(3);
        let choices: Vec<Choice> = (0..500)
            .map(|i| Choice {
                alts: (0..3).map(|_| Alt { x: vec![(0, rng.gen::<f64>())] }).collect(),
                chosen: rng.gen_range(0..3),
                cluster: i,
            })
            .collect();
        let w = vec![1.0; choices.len()];
        let fit = clogit_fit(&choices, &w, 2, &[0.0, 0.0]);
        let (model, robust) = clogit_se(&choices, &w, &fit);
        assert!(model[0].is_finite() && robust[0].is_finite());
        assert!(model[1].is_nan() && robust[1].is_nan());
    }
}
