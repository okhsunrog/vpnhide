package dev.okhsunrog.vpnhide

import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal sealed interface RootProcessResult {
    data class Completed(
        val output: String,
    ) : RootProcessResult

    data object Uncertain : RootProcessResult
}

/** Bounded caller wait, input/output and outstanding pipe ownership. Never logs command material. */
internal class RootProcessRunner {
    // One timed-out invocation must leave room for its two recovery attempts. Hung drains retain a slot.
    private val slots = Semaphore(3)

    /** For an observation worker with its own deadline: return only after the process and pipes drain. */
    fun runAndDrain(
        arguments: List<String>,
        timeoutMillis: Long = 10_000,
    ): RootProcessResult {
        require(timeoutMillis in 1..120_000)
        return execute(arguments, byteArrayOf(), System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis), 16 * 1024 * 1024)
    }

    fun run(
        arguments: List<String>,
        input: ByteArray = byteArrayOf(),
        timeoutMillis: Long = 10_000,
        outputLimit: Int = 4 * 1024 * 1024,
    ): RootProcessResult {
        require(timeoutMillis in 1..120_000 && outputLimit in 1..4 * 1024 * 1024)
        if (!slots.tryAcquire()) return RootProcessResult.Uncertain
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis)
        val result = AtomicReference<RootProcessResult>(RootProcessResult.Uncertain)
        val done = CountDownLatch(1)
        daemon("root-process") {
            try {
                result.set(execute(arguments, input, deadline, outputLimit))
            } finally {
                done.countDown()
                slots.release()
            }
        }
        return try {
            if (done.await(timeoutMillis, TimeUnit.MILLISECONDS)) result.get() else RootProcessResult.Uncertain
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            RootProcessResult.Uncertain
        }
    }

    private fun execute(
        arguments: List<String>,
        input: ByteArray,
        deadline: Long,
        outputLimit: Int,
    ): RootProcessResult {
        if (System.nanoTime() >= deadline) return RootProcessResult.Uncertain
        val process =
            try {
                ProcessBuilder(arguments).start()
            } catch (_: Exception) {
                return RootProcessResult.Uncertain
            }
        val pipes = CountDownLatch(3)
        val complete = AtomicBoolean(true)
        val output = AtomicReference("")
        pipe(pipes, complete) { process.outputStream.use { it.write(input) } }
        pipe(pipes, complete) { output.set(readBounded(process.inputStream, outputLimit, complete)) }
        pipe(pipes, complete) { readBounded(process.errorStream, 0, AtomicBoolean(true)) }
        return try {
            val finished = process.waitFor(remaining(deadline), TimeUnit.NANOSECONDS)
            val drained = finished && pipes.await(remaining(deadline), TimeUnit.NANOSECONDS)
            if (drained && complete.get() &&
                process.exitValue() == 0
            ) {
                RootProcessResult.Completed(output.get())
            } else {
                RootProcessResult.Uncertain
            }
        } catch (_: Exception) {
            RootProcessResult.Uncertain
        } finally {
            process.destroyForcibly()
            process.waitFor()
            // Caller has its own deadline. Retain the slot until inherited pipes actually close.
            pipes.await()
        }
    }

    private fun remaining(deadline: Long): Long = (deadline - System.nanoTime()).coerceAtLeast(0)

    private fun pipe(
        done: CountDownLatch,
        complete: AtomicBoolean,
        action: () -> Unit,
    ) = daemon("root-pipe") {
        try {
            action()
        } catch (_: Exception) {
            complete.set(false)
        } finally {
            done.countDown()
        }
    }

    private fun readBounded(
        stream: InputStream,
        limit: Int,
        complete: AtomicBoolean,
    ): String =
        stream.use {
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(8192)
            while (true) {
                val count = it.read(buffer)
                if (count < 0) break
                val retained = count.coerceAtMost(limit - output.size())
                output.write(buffer, 0, retained)
                if (retained != count) complete.set(false)
            }
            output.toString(Charsets.UTF_8.name())
        }

    private fun daemon(
        name: String,
        action: () -> Unit,
    ) {
        Thread(action, name).apply { isDaemon = true }.start()
    }
}
