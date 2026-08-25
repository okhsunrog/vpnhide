package dev.okhsunrog.vpnhide.ui.theme

import androidx.compose.animation.core.AnimationSpec
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MotionScheme

/**
 * Shared easing curves for the redesign's transitions.
 *
 * Curves mirror the feel of ImageToolbox's motion (Apache-2.0) — a slightly
 * overshooting primary transition plus calmer alpha/scale curves — so screen
 * and content transitions read as coordinated rather than ad-hoc tweens.
 */
object AppEasing {
    /** Primary transition curve, gently overshoots for a lively feel. */
    val FancyTransition: Easing = CubicBezierEasing(0.48f, 0.19f, 0.05f, 1.03f)

    /** Calmer curve for fade/alpha changes. */
    val Alpha: Easing = CubicBezierEasing(0.4f, 0.4f, 0.17f, 0.9f)

    /** Scale / point-to-point movement. */
    val Scale: Easing = CubicBezierEasing(0.55f, 0.55f, 0f, 1f)

    /** Near-instant response with a soft tail (e.g. press feedback). */
    val FastInvoke: Easing = CubicBezierEasing(0f, 0f, 0f, 1f)
}

/**
 * Ready-made animation specs for the expressive feel.
 *
 * The global Material 3 expressive `MotionScheme` is only public from
 * material3 1.5.0-alpha (which needs AGP 9.1+, out of reach while the native
 * build is pinned to AGP 8.13). Until then we apply the same motion explicitly:
 * spatial movement uses lively springs, effects use [AppEasing.FancyTransition]
 * tweens. Spec values adapted from ImageToolbox's `CustomMotionScheme`
 * (Apache-2.0, © 2024 T8RIN).
 */
object AppMotion {
    /** Default spatial movement (size/offset changes). */
    fun <T> defaultSpatial(): AnimationSpec<T> = tween(durationMillis = 400, easing = AppEasing.FancyTransition)

    /** Snappy spatial movement (press/expand reactions). */
    fun <T> fastSpatial(): AnimationSpec<T> = spring(dampingRatio = 0.6f, stiffness = 800f)

    /** Gentle spatial movement (large surfaces settling). */
    fun <T> slowSpatial(): AnimationSpec<T> = spring(dampingRatio = 0.8f, stiffness = 200f)

    /** Default effect (color/alpha) change. */
    fun <T> defaultEffects(): AnimationSpec<T> = spring(dampingRatio = 1.0f, stiffness = 1600f)

    /** Fast effect change. */
    fun <T> fastEffects(): AnimationSpec<T> = tween(durationMillis = 300, easing = AppEasing.FancyTransition)

    /** Slow effect change. */
    fun <T> slowEffects(): AnimationSpec<T> = tween(durationMillis = 500, easing = AppEasing.FancyTransition)
}

/**
 * Hand a spec cached as `Any` back out as one for [T].
 *
 * [MotionScheme] asks for a fresh `FiniteAnimationSpec<T>` per call site, but the
 * specs are built once and shared, so the erased element type has to be restored on
 * the way out. Sound for what is cached here: every spec below is a `tween` or a
 * `spring` with no visibility threshold, and neither ever touches a value of `T` —
 * they vectorize through the converter supplied at animation time. Do not route a
 * spec that carries `T`-typed data (a threshold, a keyframe) through this.
 *
 * One suppression, stated once, instead of six identical ones down the object.
 */
@Suppress("UNCHECKED_CAST")
private fun <T> AnimationSpec<Any>.retyped(): FiniteAnimationSpec<T> = this as FiniteAnimationSpec<T>

/**
 * Custom [MotionScheme] driving every expressive Material 3 component animation.
 *
 * Adapted from ImageToolbox's `CustomMotionScheme` (Apache-2.0, © 2024 T8RIN):
 * spatial movement uses lively springs, effects use [AppEasing.FancyTransition]
 * tweens, so motion across the app reads as deliberate and coordinated.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
val AppMotionScheme: MotionScheme =
    object : MotionScheme {
        // Single-sourced from AppMotion so the spec literals live in exactly one place.
        private val defaultSpatial = AppMotion.defaultSpatial<Any>()
        private val fastSpatial = AppMotion.fastSpatial<Any>()
        private val slowSpatial = AppMotion.slowSpatial<Any>()
        private val defaultEffects = AppMotion.defaultEffects<Any>()
        private val fastEffects = AppMotion.fastEffects<Any>()
        private val slowEffects = AppMotion.slowEffects<Any>()

        override fun <T> defaultSpatialSpec(): FiniteAnimationSpec<T> = defaultSpatial.retyped()

        override fun <T> fastSpatialSpec(): FiniteAnimationSpec<T> = fastSpatial.retyped()

        override fun <T> slowSpatialSpec(): FiniteAnimationSpec<T> = slowSpatial.retyped()

        override fun <T> defaultEffectsSpec(): FiniteAnimationSpec<T> = defaultEffects.retyped()

        override fun <T> fastEffectsSpec(): FiniteAnimationSpec<T> = fastEffects.retyped()

        override fun <T> slowEffectsSpec(): FiniteAnimationSpec<T> = slowEffects.retyped()
    }
