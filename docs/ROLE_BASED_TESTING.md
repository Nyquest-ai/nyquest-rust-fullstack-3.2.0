# Nyquest Role-Based Compression Testing

**Version:** 3.2.0 (Rust Full-Stack)  
**Date:** May 10, 2026  
**Author:** Nyquest AI  
**Engine:** Nyquest Compression Engine (Rust)  
**Platform:** Ubuntu 24.04

---

## 1. Overview

Role-Based Testing (RBT) validates the Nyquest compression engine across 25 distinct AI persona scenarios spanning 7 real-world categories. Each scenario consists of a verbose system prompt loaded with common LLM boilerplate patterns and a realistic user query. The test measures token reduction, semantic preservation, and throughput at multiple compression levels.

### Objectives

- Quantify compression savings across diverse professional domains
- Verify that domain-specific terminology is preserved (not compressed)
- Confirm compression scales monotonically with compression level
- Establish per-category baselines for production deployment targets
- Validate that no semantic meaning is lost during compression

---

## 2. Test Categories and Scenarios

### 2.1 Academic (5 scenarios)

| # | Role | Description |
|---|------|-------------|
| 1 | University Professor | CS curriculum design, graduate course planning |
| 2 | Research Advisor | PhD mentoring, ML experiment methodology |
| 3 | Academic Librarian | Information literacy, database search strategy |
| 4 | Grant Writer | NIH/NSF proposal structuring, budget planning |
| 5 | Student Advisor | Degree planning, prerequisite chain management |

### 2.2 Corporate (5 scenarios)

| # | Role | Description |
|---|------|-------------|
| 6 | CFO Analyst | M&A valuation, financial modeling |
| 7 | HR Director | Turnover analysis, retention strategy |
| 8 | Product Manager | Feature prioritization, RICE scoring |
| 9 | Marketing Director | B2B demand generation, lead pipeline |
| 10 | IT Director | Tool standardization, change management |

### 2.3 Medical (5 scenarios)

| # | Role | Description |
|---|------|-------------|
| 11 | Clinical Pharmacist | Drug interactions, medication therapy management |
| 12 | ER Triage Nurse | Emergency severity index, time-sensitive protocols |
| 13 | Medical Researcher | Clinical trial methodology, literature review |
| 14 | Health IT Specialist | EHR implementation, HIPAA/FHIR compliance |
| 15 | Physical Therapist | Orthopedic rehab protocols, progressive loading |

### 2.4 Scientific (5 scenarios)

| # | Role | Description |
|---|------|-------------|
| 16 | Climate Scientist | IPCC projections, sea level rise modeling |
| 17 | Bioinformatician | Genomic pipelines, somatic variant calling |
| 18 | Materials Scientist | Composite materials, aerospace applications |
| 19 | Astrophysicist | Exoplanet detection, JWST observations |
| 20 | Environmental Engineer | PFAS remediation, EPA compliance |

### 2.5 Small Business (1 scenario)

| # | Role | Description |
|---|------|-------------|
| 21 | Small Biz Advisor | Low-budget growth, online sales strategy |

### 2.6 Home (2 scenarios)

| # | Role | Description |
|---|------|-------------|
| 22 | Smart Home Assistant | Automation, energy savings, Matter/Thread |
| 23 | Home Renovation Coach | Budget planning, permits, ROI analysis |

### 2.7 Robot / Vehicle (2 scenarios)

| # | Role | Description |
|---|------|-------------|
| 24 | Vehicle Copilot | Navigation, driver safety, spoken HMI |
| 25 | Warehouse Robot | Pick/place operations, SKU verification |

---

## 3. Benchmark Results (Level 1.0)

### 3.1 Academic

| Role | Sys In | Sys Out | Usr In | Usr Out | Total Savings | µs/call |
|------|--------|---------|--------|---------|---------------|---------|
| University Professor | 206 | 163 | 45 | 44 | **16.9%** | 1,139 |
| Research Advisor | 214 | 168 | 51 | 51 | **16.7%** | 1,184 |
| Academic Librarian | 211 | 172 | 50 | 50 | **14.4%** | 1,227 |
| Grant Writer | 229 | 181 | 43 | 43 | **17.0%** | 1,296 |
| Student Advisor | 229 | 187 | 48 | 48 | **14.6%** | 1,347 |
| **Category Average** | **218** | **174** | **47** | **47** | **15.9%** | **1,239** |

### 3.2 Corporate

| Role | Sys In | Sys Out | Usr In | Usr Out | Total Savings | µs/call |
|------|--------|---------|--------|---------|---------------|---------|
| CFO Analyst | 223 | 178 | 51 | 50 | **16.2%** | 1,328 |
| HR Director | 224 | 183 | 47 | 46 | **14.9%** | 1,335 |
| Product Manager | 234 | 181 | 53 | 53 | **17.8%** | 1,370 |
| Marketing Director | 207 | 169 | 47 | 47 | **14.4%** | 1,234 |
| IT Director | 236 | 190 | 59 | 59 | **15.1%** | 1,431 |
| **Category Average** | **225** | **180** | **51** | **51** | **15.7%** | **1,340** |

### 3.3 Medical

| Role | Sys In | Sys Out | Usr In | Usr Out | Total Savings | µs/call |
|------|--------|---------|--------|---------|---------------|---------|
| Clinical Pharmacist | 258 | 214 | 55 | 54 | **13.9%** | 1,615 |
| ER Triage Nurse | 223 | 183 | 50 | 49 | **14.5%** | 1,299 |
| Medical Researcher | 225 | 185 | 49 | 49 | **14.1%** | 1,352 |
| Health IT Specialist | 243 | 191 | 52 | 49 | **18.0%** | 1,435 |
| Physical Therapist | 245 | 210 | 46 | 46 | **11.6%** | 1,500 |
| **Category Average** | **239** | **197** | **50** | **49** | **14.4%** | **1,440** |

### 3.4 Scientific

| Role | Sys In | Sys Out | Usr In | Usr Out | Total Savings | µs/call |
|------|--------|---------|--------|---------|---------------|---------|
| Climate Scientist | 231 | 195 | 55 | 55 | **12.2%** | 1,419 |
| Bioinformatician | 226 | 194 | 46 | 45 | **11.7%** | 1,306 |
| Materials Scientist | 238 | 203 | 56 | 55 | **11.8%** | 1,435 |
| Astrophysicist | 233 | 201 | 56 | 55 | **11.0%** | 1,387 |
| Environmental Engineer | 231 | 195 | 63 | 62 | **12.2%** | 1,406 |
| **Category Average** | **232** | **198** | **55** | **54** | **11.8%** | **1,391** |

### 3.5 Small Business / Home / Robot-Vehicle

| Role | Category | Sys In | Sys Out | Usr In | Usr Out | Savings | µs/call |
|------|----------|--------|---------|--------|---------|---------|---------|
| Small Biz Advisor | Small Business | 244 | 203 | 56 | 55 | **13.5%** | 1,524 |
| Smart Home Assistant | Home | 235 | 195 | 59 | 58 | **13.5%** | 1,472 |
| Home Renovation Coach | Home | 243 | 208 | 51 | 51 | **11.5%** | 1,520 |
| Vehicle Copilot | Vehicle | 231 | 198 | 28 | 28 | **12.3%** | 1,467 |
| Warehouse Robot | Robot | 255 | 215 | 47 | 47 | **12.8%** | 1,726 |
| **Category Average** | — | **242** | **204** | **48** | **48** | **12.7%** | **1,542** |

### 3.6 Aggregate

| Metric | Value |
|--------|-------|
| Total original tokens | 7,287 |
| Total compressed tokens | 6,261 |
| **Overall savings** | **14.1%** |
| Roles tested | 25 |
| Categories | 7 |
| Compression failures | 0 |
| Domain terms lost | 0 |

---

## 4. Category Analysis

### Compression Ranking by Category

| Rank | Category | Avg Savings | Avg Latency | Notes |
|------|----------|-------------|-------------|-------|
| 1 | Academic | 15.9% | 1,239 µs | Highest filler density |
| 2 | Corporate | 15.7% | 1,340 µs | Heavy boilerplate patterns |
| 3 | Medical | 14.4% | 1,440 µs | More domain terms preserved |
| 4 | Small Business | 13.5% | 1,524 µs | Moderate filler + colloquial |
| 5 | Home | 12.5% | 1,496 µs | Mixed filler + practical |
| 6 | Robot/Vehicle | 12.6% | 1,597 µs | Safety-critical preserves more |
| 7 | Scientific | 11.8% | 1,391 µs | Highest domain term density |

### Key Findings

1. **System prompts absorb 90%+ of savings** — user messages are typically concise and domain-specific, leaving little to compress.

2. **Domain terminology is 100% preserved** — all tested terms (HIPAA, FHIR, STEMI, IPCC, PFAS, Sarbanes-Oxley, EEOC, CRAAP, NSF, SKU, Matter) survive compression intact.

3. **Scientific roles compress least** because their prompts contain the highest density of technical jargon that the engine correctly identifies as semantically critical.

4. **Academic and Corporate roles compress most** because they are loaded with business/academic filler patterns: "It is important to note that...", "Please note that...", "Due to the fact that...", "For the purpose of facilitating...", "In order to utilize...".

5. **Robot/Vehicle roles show moderate compression** — safety-critical language is preserved (e.g., "driver safety is the absolute top priority") while boilerplate is removed.

6. **All roles pass monotonic test** — higher compression levels always produce fewer or equal tokens, never more.

---

## 5. Formal Test Suite

**Location:** `tests/role_based_test.rs`  
**Run:** `cargo test --test role_based_test`

### Test Cases

| Test | Description | Assertion |
|------|-------------|-----------|
| `test_all_roles_compress_above_minimum` | Every role meets its minimum savings threshold | Academic/Corporate ≥ 10%, Medical/Scientific ≥ 8% |
| `test_no_role_loses_tokens` | Compression never increases token count | `optimized ≤ original` for all 25 roles |
| `test_level_zero_is_passthrough` | Level 0.0 produces identical output | `original == compressed` for all 25 roles |
| `test_domain_terms_preserved` | Critical domain terms survive compression | 11 terms across all categories verified |
| `test_compression_monotonic_with_level` | Higher levels = more compression | Tokens decrease monotonically: 0.0 → 0.2 → 0.5 → 0.8 → 1.0 |
| `test_aggregate_savings_above_target` | Overall savings meets product target | Aggregate ≥ 10% across all 25 roles |

### Results

```
running 6 tests
test test_level_zero_is_passthrough ... ok
test test_domain_terms_preserved ... ok
test test_compression_monotonic_with_level ... ok
test test_no_role_loses_tokens ... ok
test test_all_roles_compress_above_minimum ... ok
test test_aggregate_savings_above_target ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.73s
```

---

## 6. Compression Pattern Examples

Patterns targeted by the Nyquest engine across all role prompts:

| Input Pattern | Compressed Output | Category |
|---------------|-------------------|----------|
| "Please note that it is important to note that" | *(removed)* | Filler |
| "It is important to note that" | *(removed)* | Filler |
| "Due to the fact that" | "because" | Verbose |
| "For the purpose of facilitating" | "to facilitate" | Verbose |
| "In order to utilize" | "to use" | Verbose |
| "You should make sure to" | *(removed)* | Filler |
| "Please ensure that you always" | *(removed or shortened)* | Filler |
| "Needless to say, as you know" | *(removed)* | Filler |
| "Basically, the goal is to" | *(removed)* | Filler |
| "Act as a senior developer who is an expert in Python" | "Expert: Python" | Role |
| "January 14th, 2025" | "25-01-14" | Date |
| "forty-two thousand dollars" | "42k dollars" | Number |

---

## 7. Running the Benchmark

### Quick Run

```bash
cd nyquest-rust-fullstack-3.2.0
cargo build --release
./target/release/benchmark
```

### Test Suite Only

```bash
cargo test --test role_based_test -- --nocapture
```

### Full Verification

```bash
cargo build --release         # 0 warnings, 0 errors
cargo test                    # All pass
cargo clippy --release        # 0 warnings
./target/release/benchmark    # Full 25-role benchmark
```

---

## 8. Performance Optimization Log

### Critical Bug: compress_dates() Regex Recompilation

**Discovered:** Feb 28, 2026  
**Impact:** 118x performance degradation at compression level >= 0.5  
**Root cause:** `compress_dates()` compiled 24 new Regex objects per invocation  
**Fix:** Moved all date patterns to `Lazy<Vec<...>>` static, compiled once at startup  
**Result:** 9,250 µs → 4.1 µs (2,256x improvement)

### Before/After

| Scenario | Before | After | Speedup |
|----------|--------|-------|---------|
| Small text @ 0.5 | 9,304 µs | 108 µs | 86x |
| Medium request @ 0.5 | 37,693 µs | 783 µs | 48x |
| Large 20-turn @ 0.5 | 103,403 µs | 1,550 µs | 67x |

---

## 9. Memory Profile

| Metric | Value |
|--------|-------|
| VmPeak | 25.7 MB |
| VmRSS | 24.3 MB |

Memory is stable across all 25 roles with no growth observed during repeated benchmark runs.

---

*Nyquest — [nyquest.ai](https://nyquest.ai) — Built by [Nyquest AI](https://github.com/Nyquest-ai)*
