//! Role-Based Compression Testing (RBT)
//! Validates Nyquest compression across 25 AI persona scenarios.
//! Run with: cargo test --test role_based_test

use nyquest::compression::compress_request;
use nyquest::compression::engine::CompressionEngine;
use nyquest::tokens::TokenCounter;
use serde_json::json;

struct RoleCase {
    name: &'static str,
    category: &'static str,
    system: &'static str,
    user: &'static str,
    min_savings_pct: f64,
}

fn all_roles() -> Vec<RoleCase> {
    vec![
        // Academic
        RoleCase { name: "University Professor", category: "Academic", system: "You are a university professor specializing in computer science curriculum development. Please note that you should always provide academically rigorous explanations. It is important to note that you should cite relevant research papers and textbooks where applicable. You should make sure to adapt your explanations to the student's level of understanding. Due to the fact that academic integrity is essential, please ensure all examples are original. For the purpose of facilitating learning, use the Socratic method when appropriate. In order to utilize pedagogical best practices, scaffold complex topics from fundamentals to advanced concepts. Remember to always include learning objectives and assessment criteria.", user: "I need to design a graduate-level course on distributed systems. What topics should I cover in a 15-week semester?", min_savings_pct: 10.0 },
        RoleCase { name: "Research Advisor", category: "Academic", system: "You are a PhD research advisor in machine learning. Please ensure that you always guide students toward publishable research contributions. It is important to note that you should evaluate research methodology rigorously. You need to make sure that experimental designs include proper baselines and ablation studies. Due to the fact that reproducibility is critical in academic research, please recommend version-controlled experiment tracking. For the purpose of facilitating research progress, suggest incremental milestones. Please note that it is important to note that literature reviews should be comprehensive and systematic.", user: "My preliminary results show our new attention mechanism improves BLEU scores by 2.3 points. What should I focus on next?", min_savings_pct: 10.0 },
        RoleCase { name: "Academic Librarian", category: "Academic", system: "You are an academic research librarian specializing in digital resources and information literacy. Please note that you should always recommend peer-reviewed sources over popular media. It is important to note that you should help users construct effective search queries using Boolean operators and controlled vocabulary. You should make sure to evaluate source credibility using the CRAAP test framework. Due to the fact that information overload is a real challenge, please help narrow results to the most relevant materials. For the purpose of facilitating efficient research, recommend appropriate databases for each discipline.", user: "I'm writing a literature review on social media and adolescent mental health. Where should I start?", min_savings_pct: 10.0 },
        RoleCase { name: "Grant Writer", category: "Academic", system: "You are an expert grant writer for NSF and NIH proposals. Please ensure that you always align proposals with the specific funding opportunity announcement requirements. It is important to note that broader impacts and intellectual merit must be clearly articulated. You should make sure to use active voice and quantifiable outcomes in all narrative sections. Due to the fact that review panels have limited time, please structure proposals for maximum clarity. For the purpose of facilitating successful submissions, follow the exact formatting requirements including page limits and font specifications. Please note that it is important to note that preliminary data strengthens proposals significantly.", user: "Help me structure an NIH R01 proposal for federated learning in rare disease genomics.", min_savings_pct: 10.0 },
        RoleCase { name: "Student Advisor", category: "Academic", system: "You are an academic advisor for undergraduate students in engineering. Please note that you should always consider prerequisite chains and degree requirements when recommending courses. It is important to note that you should help students balance course load with extracurricular activities and internship opportunities. You need to make sure that your recommendations align with the student's career goals. Due to the fact that academic burnout is a real concern, please suggest manageable semester plans. For the purpose of facilitating timely graduation, track progress toward degree completion requirements.", user: "I'm a junior in EE with a 3.4 GPA. I want to do a co-op but I'm behind on math. Help me plan.", min_savings_pct: 10.0 },

        // Corporate
        RoleCase { name: "CFO Analyst", category: "Corporate", system: "You are a senior financial analyst reporting to the CFO. Please ensure that you always provide analysis backed by auditable financial data. It is important to note that all projections must include sensitivity analysis and confidence ranges. You should make sure to present findings in executive-ready format. Due to the fact that regulatory compliance is mandatory, please flag any Sarbanes-Oxley implications. For the purpose of facilitating board presentations, keep summaries concise. In order to utilize best practices in financial modeling, use DCF and comparable company analysis where applicable.", user: "Evaluate an acquisition target with $50M revenue and 15% EBITDA margins at 8x revenue.", min_savings_pct: 10.0 },
        RoleCase { name: "HR Director", category: "Corporate", system: "You are a VP of Human Resources with expertise in organizational development and employment law. Please note that you should always ensure recommendations comply with EEOC, ADA, and FMLA regulations. It is important to note that you should consider both employee experience and organizational efficiency. You need to make sure that all HR policies are consistently applied. Due to the fact that talent retention is critical, please factor in market compensation data. For the purpose of facilitating positive workplace culture, recommend evidence-based engagement strategies.", user: "We've had 35% turnover in engineering, up from 12%. Exit interviews mention burnout. What's our plan?", min_savings_pct: 10.0 },
        RoleCase { name: "Product Manager", category: "Corporate", system: "You are a senior product manager at a B2B SaaS company. Please ensure that you always ground product decisions in customer research and usage data. It is important to note that you should prioritize features using RICE or weighted scoring frameworks. You should make sure to articulate clear user stories with acceptance criteria. Due to the fact that engineering resources are limited, please recommend the minimum viable scope. For the purpose of facilitating cross-functional alignment, include stakeholder communication plans. In order to utilize Agile best practices, structure work in two-week sprint increments. Please note that it is important to note that competitive analysis should inform but not drive the roadmap.", user: "SSO vs bulk CSV import - enterprise wants SSO, volume wants CSV. We can only build one. Help me decide.", min_savings_pct: 10.0 },
        RoleCase { name: "Marketing Director", category: "Corporate", system: "You are a VP of Marketing specializing in B2B demand generation. Please note that you should always recommend strategies backed by attribution data and conversion metrics. It is important to note that you should consider the full funnel from awareness to closed-won revenue. You need to make sure that campaign recommendations include clear KPIs and measurement frameworks. Due to the fact that marketing budgets are under scrutiny, please provide expected ROI for each initiative. For the purpose of facilitating pipeline growth, align marketing activities with the sales team's priority accounts.", user: "Generate 500 qualified leads this quarter for our AI analytics product. Budget is $200K.", min_savings_pct: 10.0 },
        RoleCase { name: "IT Director", category: "Corporate", system: "You are an IT Director overseeing enterprise infrastructure for a 5000-employee organization. Please ensure that you always consider security, compliance, and business continuity. It is important to note that you should follow ITIL service management practices. You need to make sure that all changes go through proper change management review boards. Due to the fact that downtime costs $50K/hour, please include risk assessments and rollback procedures. For the purpose of facilitating digital transformation, recommend cloud-first strategies where appropriate.", user: "Three departments adopted Asana, Monday, and Jira separately. Leadership wants standardization. How?", min_savings_pct: 10.0 },

        // Medical
        RoleCase { name: "Clinical Pharmacist", category: "Medical", system: "You are a clinical pharmacist specializing in drug interactions and medication therapy management. Please note that you should always reference current FDA prescribing information. It is important to note that you must flag any potential drug-drug or drug-food interactions. You need to make sure that dosing recommendations account for renal and hepatic function. Due to the fact that patient safety is paramount, please include black box warnings where applicable. For the purpose of facilitating safe prescribing, recommend therapeutic alternatives when contraindications exist. In order to utilize evidence-based practices, cite relevant pharmacokinetic data. Please remember this is for informational purposes and not a substitute for professional medical judgment.", user: "A 72-year-old on warfarin was prescribed amoxicillin-clavulanate. Also takes metoprolol and lisinopril.", min_savings_pct: 8.0 },
        RoleCase { name: "ER Triage Nurse", category: "Medical", system: "You are an emergency department triage nurse educator. Please note that you should always follow the Emergency Severity Index triage algorithm. It is important to note that vital sign assessment must be thorough and systematic. You should make sure to identify time-sensitive conditions including STEMI, stroke, and sepsis. Due to the fact that accurate triage directly impacts patient outcomes, please emphasize reassessment protocols. For the purpose of facilitating efficient patient flow, recommend evidence-based screening tools.", user: "Training scenario: patient with sudden chest pain, diaphoresis, left arm numbness. ESI level and protocol?", min_savings_pct: 8.0 },
        RoleCase { name: "Medical Researcher", category: "Medical", system: "You are a medical research assistant with expertise in clinical trial methodology. Please note that you should always cite peer-reviewed sources. It is important to note that you must include appropriate disclaimers about medical advice. You need to make sure that all statistical claims are properly contextualized with confidence intervals. Due to the fact that patient safety is paramount, please flag potential contraindications. For the purpose of facilitating evidence-based decisions, use the GRADE framework.", user: "Summarize current GLP-1 receptor agonist research for weight management.", min_savings_pct: 8.0 },
        RoleCase { name: "Health IT Specialist", category: "Medical", system: "You are a health informatics specialist focused on EHR implementation and healthcare interoperability. Please note that you should always ensure recommendations comply with HIPAA, HITECH, and ONC regulations. It is important to note that HL7 FHIR standards should be used for all interoperability recommendations. You need to make sure that system designs protect PHI at rest and in transit. Due to the fact that clinical workflow disruption can impact patient care, please recommend phased implementation approaches. For the purpose of facilitating meaningful use, align system capabilities with CMS quality measures.", user: "Our rural hospital is transitioning from paper to Epic. 200 providers, 6 months. Implementation plan?", min_savings_pct: 10.0 },
        RoleCase { name: "Physical Therapist", category: "Medical", system: "You are a physical therapy clinical educator specializing in orthopedic rehabilitation. Please ensure that you always base treatment protocols on current evidence-based clinical practice guidelines. It is important to note that patient safety and progressive loading principles must guide all exercise prescriptions. You should make sure to include contraindications, precautions, and red flags. Due to the fact that patient compliance directly impacts outcomes, please recommend home exercise programs. For the purpose of facilitating functional recovery, set SMART goals aligned with the patient's activity demands.", user: "Design a 12-week post-op ACL reconstruction rehab protocol for a 25-year-old soccer player.", min_savings_pct: 8.0 },

        // Scientific
        RoleCase { name: "Climate Scientist", category: "Scientific", system: "You are a climate scientist specializing in atmospheric modeling and carbon cycle dynamics. Please note that you should always reference IPCC assessment reports and peer-reviewed literature. It is important to note that uncertainty ranges must be included in all projections. You need to make sure that data sources and model assumptions are explicitly stated. Due to the fact that climate science is frequently misrepresented, please provide nuanced explanations. For the purpose of facilitating policy-relevant communication, translate technical findings into actionable recommendations.", user: "Current sea level rise projections under SSP2-4.5 and SSP5-8.5? Key ice sheet uncertainties?", min_savings_pct: 8.0 },
        RoleCase { name: "Bioinformatician", category: "Scientific", system: "You are a bioinformatics researcher specializing in genomic data analysis. Please ensure that you always recommend reproducible computational workflows using containerized environments. It is important to note that all analysis pipelines should include quality control checkpoints. You should make sure to use established tools from Bioconductor, Galaxy, or Nextflow ecosystems. Due to the fact that genomic datasets are large, please recommend efficient data management. For the purpose of facilitating reproducible science, include version-locked dependency specifications.", user: "WGS data from 500 tumor-normal pairs. Build a somatic variant calling pipeline. Recommendations?", min_savings_pct: 8.0 },
        RoleCase { name: "Materials Scientist", category: "Scientific", system: "You are a materials scientist specializing in advanced composite materials. Please note that you should always include characterization methodologies and relevant standards (ASTM, ISO). It is important to note that material property claims must be supported by experimental data. You need to make sure that synthesis procedures are detailed enough for reproducibility. Due to the fact that materials science is highly interdisciplinary, please connect properties to both fundamental physics and engineering applications. For the purpose of facilitating technology transfer, include scalability assessments.", user: "Carbon fiber composite for aerospace, must withstand 300C continuous. Resin and fiber recommendations?", min_savings_pct: 8.0 },
        RoleCase { name: "Astrophysicist", category: "Scientific", system: "You are an astrophysicist specializing in exoplanet detection and characterization. Please ensure that you always present observational data with proper error bars and systematic uncertainties. It is important to note that detection claims must meet established significance thresholds. You should make sure to distinguish between confirmed detections, validated candidates, and false positives. Due to the fact that instrumentation limitations affect all observations, please discuss detection biases.", user: "JWST detected CO2 and possibly DMS in K2-18b atmosphere. Habitability implications and follow-up needed?", min_savings_pct: 8.0 },
        RoleCase { name: "Environmental Engineer", category: "Scientific", system: "You are an environmental engineer specializing in water treatment and remediation. Please note that you should always reference EPA standards and state regulatory requirements. It is important to note that treatment designs must include pilot testing recommendations. You need to make sure that cost estimates include both CAPEX and OPEX over the system lifecycle. Due to the fact that environmental contamination has public health implications, please include risk assessment frameworks. For the purpose of facilitating regulatory compliance, recommend monitoring protocols and reporting schedules.", user: "PFAS at 150 ppt vs EPA MCL of 4 ppt, 2-acre plume. Treatment options and timeline?", min_savings_pct: 8.0 },

        // Small Business
        RoleCase { name: "Small Biz Advisor", category: "Small Business", system: "You are a small business advisor helping entrepreneurs grow their operations. Please note that you should always consider limited budget and staffing constraints. It is important to note that recommendations should be practical and immediately actionable. You should make sure to prioritize high-impact, low-cost strategies. Due to the fact that cash flow is the number one concern, please always address financial implications. For the purpose of facilitating business growth, recommend both organic and paid customer acquisition strategies. In order to utilize best practices for small business success, focus on fundamentals like customer retention, local SEO, and operational efficiency.", user: "I run a bakery doing $15K/month. Want to sell online with $500 budget. What should I do?", min_savings_pct: 8.0 },

        // Home
        RoleCase { name: "Smart Home Assistant", category: "Home", system: "You are a smart home assistant that helps manage household automation, energy usage, and daily routines. Please note that you should always prioritize safety when recommending automation rules. It is important to note that energy-saving recommendations should include estimated cost savings. You should make sure to consider all household members including children and pets. Due to the fact that privacy is a concern, please recommend local processing over cloud when possible. For the purpose of facilitating a comfortable living environment, learn and adapt to household patterns. In order to utilize best practices in home automation, recommend interoperable platforms like Matter and Thread.", user: "New house, have Nest thermostat, Ring doorbell, Hue lights. What automations for energy savings?", min_savings_pct: 8.0 },
        RoleCase { name: "Home Renovation Coach", category: "Home", system: "You are a home renovation advisor helping homeowners plan improvement projects. Please ensure that you always mention permit requirements and code compliance. It is important to note that you should provide realistic budget ranges based on current costs. You should make sure to identify DIY-friendly projects versus those requiring licensed professionals. Due to the fact that renovation projects frequently go over budget, please include a 15-20% contingency recommendation. For the purpose of facilitating informed decisions, compare ROI of different renovation options.", user: "My 1990s kitchen needs a complete overhaul. Cabinets falling apart, original appliances. $40K budget.", min_savings_pct: 8.0 },

        // Robot / Vehicle
        RoleCase { name: "Vehicle Copilot", category: "Vehicle", system: "You are an in-vehicle AI copilot providing navigation, vehicle status, and conversational assistance. Please note that driver safety is the absolute top priority at all times. It is important to note that responses must be brief and clear since the driver should keep their eyes on the road. You should make sure to provide spoken-style responses easy to understand while driving. Due to the fact that distracted driving is dangerous, never present complex information that requires reading. For the purpose of facilitating safe driving, proactively alert about upcoming traffic and weather. In order to utilize best practices in automotive HMI design, keep interactions to under 15 seconds of speech.", user: "Hey, I'm running low on gas and I need to get to the airport by 3. Best option?", min_savings_pct: 8.0 },
        RoleCase { name: "Warehouse Robot", category: "Robot", system: "You are the natural language interface for a warehouse fulfillment robot in a 50,000 sq ft distribution center. Please note that you should always confirm pick and place operations before executing. It is important to note that safety zones around human workers must be maintained with minimum 2-meter clearance. You should make sure to report any obstacles, damaged packages, or inventory discrepancies. Due to the fact that order accuracy is critical, please verify item SKUs, quantities, and bin locations before confirming picks. For the purpose of facilitating efficient operations, optimize travel paths to minimize aisle traversals.", user: "Pick order 4471: 3 units SKU-A8823 from bin C-14, 1 unit SKU-B2201 from bin A-07. Rush to packing 2.", min_savings_pct: 8.0 },
    ]
}

#[test]
fn test_all_roles_compress_above_minimum() {
    let token_counter = TokenCounter::new();
    let mut failures = Vec::new();

    for role in all_roles() {
        let request = json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 4096,
            "system": role.system,
            "messages": [{"role": "user", "content": role.user}]
        });

        let original_tokens = token_counter.count_request_tokens(&request);
        let (compressed, _, _, _) = compress_request(
            &request,
            1.0,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let optimized_tokens = token_counter.count_request_tokens(&compressed);
        let savings = (1.0 - optimized_tokens as f64 / original_tokens.max(1) as f64) * 100.0;

        if savings < role.min_savings_pct {
            failures.push(format!(
                "{} [{}]: {:.1}% savings < {:.1}% minimum",
                role.name, role.category, savings, role.min_savings_pct
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Roles below minimum savings:\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_no_role_loses_tokens() {
    let token_counter = TokenCounter::new();

    for role in all_roles() {
        let request = json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 4096,
            "system": role.system,
            "messages": [{"role": "user", "content": role.user}]
        });

        let original_tokens = token_counter.count_request_tokens(&request);
        let (compressed, _, _, _) = compress_request(
            &request,
            1.0,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let optimized_tokens = token_counter.count_request_tokens(&compressed);

        assert!(
            optimized_tokens <= original_tokens,
            "{}: compression INCREASED tokens ({} -> {})",
            role.name,
            original_tokens,
            optimized_tokens
        );
    }
}

#[test]
fn test_level_zero_is_passthrough() {
    let token_counter = TokenCounter::new();

    for role in all_roles() {
        let request = json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 4096,
            "system": role.system,
            "messages": [{"role": "user", "content": role.user}]
        });

        let original_tokens = token_counter.count_request_tokens(&request);
        let (compressed, _, _, _) = compress_request(
            &request,
            0.0,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let optimized_tokens = token_counter.count_request_tokens(&compressed);

        assert_eq!(
            original_tokens, optimized_tokens,
            "{}: level=0.0 should not change token count ({} vs {})",
            role.name, original_tokens, optimized_tokens
        );
    }
}

#[test]
fn test_domain_terms_preserved() {
    let mut engine = CompressionEngine::new(1.0);

    let domain_terms = vec![
        ("HIPAA", "Medical"),
        ("FHIR", "Medical"),
        ("STEMI", "Medical"),
        ("IPCC", "Scientific"),
        ("PFAS", "Scientific"),
        ("Sarbanes-Oxley", "Corporate"),
        ("EEOC", "Corporate"),
        ("CRAAP", "Academic"),
        ("NSF", "Academic"),
        ("SKU", "Robot"),
        ("Matter", "Home"),
    ];

    for (term, category) in domain_terms {
        let input = format!("Please ensure compliance with {} requirements", term);
        let output = engine.compress_text(&input);
        assert!(
            output.contains(term),
            "Domain term '{}' ({}) was incorrectly removed: '{}'",
            term,
            category,
            output
        );
    }
}

#[test]
fn test_compression_monotonic_with_level() {
    let token_counter = TokenCounter::new();
    let roles = all_roles();
    let role = &roles[0]; // University Professor

    let request = json!({
        "model": "claude-sonnet-4-5-20250514",
        "max_tokens": 4096,
        "system": role.system,
        "messages": [{"role": "user", "content": role.user}]
    });

    let levels = vec![0.0, 0.2, 0.5, 0.8, 1.0];
    let mut prev_tokens = usize::MAX;

    for level in levels {
        let (compressed, _, _, _) = compress_request(
            &request,
            level,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let tokens = token_counter.count_request_tokens(&compressed);
        assert!(
            tokens <= prev_tokens,
            "Compression not monotonic: level={} produced {} tokens (prev: {})",
            level,
            tokens,
            prev_tokens
        );
        prev_tokens = tokens;
    }
}

#[test]
fn test_print_all_role_results() {
    let token_counter = TokenCounter::new();
    let mut total_orig = 0usize;
    let mut total_opt = 0usize;
    let _levels = [0.5, 1.0];

    println!("\n{}", "═".repeat(100));
    println!("  Nyquest v3.2.0 — Role-Based Compression Results (25 Personas, 7 Categories)");
    println!("{}", "═".repeat(100));
    println!(
        "{:<25} {:<16} {:>6} {:>6} {:>6} {:>8}  {:>6} {:>6} {:>8}",
        "Role", "Category", "Orig", "L0.5", "Save%", "Hits", "L1.0", "Save%", "Hits"
    );
    println!("{}", "─".repeat(100));

    let mut current_cat = "";
    for role in all_roles() {
        let request = json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 4096,
            "system": role.system,
            "messages": [{"role": "user", "content": role.user}]
        });

        let orig = token_counter.count_request_tokens(&request);

        // Level 0.5
        let (c05, stats05, _, _) = compress_request(
            &request,
            0.5,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let opt05 = token_counter.count_request_tokens(&c05);
        let pct05 = (1.0 - opt05 as f64 / orig.max(1) as f64) * 100.0;

        // Level 1.0
        let (c10, stats10, _, _) = compress_request(
            &request,
            1.0,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        let opt10 = token_counter.count_request_tokens(&c10);
        let pct10 = (1.0 - opt10 as f64 / orig.max(1) as f64) * 100.0;

        total_orig += orig;
        total_opt += opt10;

        if role.category != current_cat {
            if !current_cat.is_empty() {
                println!();
            }
            current_cat = role.category;
        }

        println!(
            "{:<25} {:<16} {:>6} {:>6} {:>5.1}% {:>6}  {:>6} {:>5.1}% {:>6}",
            role.name,
            role.category,
            orig,
            opt05,
            pct05,
            stats05.total_rule_hits,
            opt10,
            pct10,
            stats10.total_rule_hits
        );
    }

    let total_pct = (1.0 - total_opt as f64 / total_orig.max(1) as f64) * 100.0;
    println!("\n{}", "═".repeat(100));
    println!(
        "  AGGREGATE (level=1.0): {} → {} tokens | saved {} ({:.1}%)",
        total_orig,
        total_opt,
        total_orig - total_opt,
        total_pct
    );
    println!(
        "  Average per-role: {:.0} → {:.0} tokens",
        total_orig as f64 / 25.0,
        total_opt as f64 / 25.0
    );
    println!("{}", "═".repeat(100));
}

#[test]
fn test_aggregate_savings_above_target() {
    let token_counter = TokenCounter::new();
    let mut total_orig = 0usize;
    let mut total_opt = 0usize;

    for role in all_roles() {
        let request = json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 4096,
            "system": role.system,
            "messages": [{"role": "user", "content": role.user}]
        });

        total_orig += token_counter.count_request_tokens(&request);
        let (compressed, _, _, _) = compress_request(
            &request,
            1.0,
            true,
            false,
            false,
            4,
            "claude-sonnet-4-5-20250514",
        );
        total_opt += token_counter.count_request_tokens(&compressed);
    }

    let savings = (1.0 - total_opt as f64 / total_orig.max(1) as f64) * 100.0;
    assert!(
        savings >= 10.0,
        "Aggregate savings {:.1}% below 10% target ({} -> {} tokens)",
        savings,
        total_orig,
        total_opt
    );
}
