# Says which processor instructions the speech library may be built with on
# Windows. Dot-sourced by the build scripts and run by CI before it builds:
#
#   . scripts\win-cpu-floor.ps1
#
# Left alone, the library (transcribe.cpp, built from source by its crate)
# tunes itself to the processor of the machine that builds it. Built on a
# machine with AVX-512, as some of GitHub's are, the app then stops with
# "illegal instruction" on every PC without it, which is most of them; and a
# build kept in CI's cache fails the same way on the next machine that runs
# its tests.
#
# So it's pinned to what nearly every PC since 2013 has (AVX2 and its
# companions), the same as the fast local-model helper in package-win.ps1.
# The app checks for these before it uses the library
# (`speech_library_runs_here` in crates\splashdown-providers) and does
# without on-device speech on an older processor, rather than crash.
$floor = "-DGGML_NATIVE=OFF " +
    "-DGGML_SSE42=ON -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON -DGGML_BMI2=ON " +
    "-DGGML_AVX_VNNI=OFF -DGGML_AVX512=OFF -DGGML_AVX512_VBMI=OFF -DGGML_AVX512_VNNI=OFF -DGGML_AVX512_BF16=OFF"
# Something already chosen by whoever is building wins.
if (-not $env:TRANSCRIBE_CMAKE_ARGS) { $env:TRANSCRIBE_CMAKE_ARGS = $floor }
# In GitHub Actions, for the steps after this one too.
if ($env:GITHUB_ENV) {
    "TRANSCRIBE_CMAKE_ARGS=$($env:TRANSCRIBE_CMAKE_ARGS)" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
}
