/**
 * Nyquest Compression Engine — JavaScript Port
 * 532 regex rules across 19 categories, 3 tiers.
 * Mirrors the Rust engine in src/compression/rules.rs
 *
 * Usage:
 *   const engine = new NyquestCompressor(0.7);
 *   const { text, stats } = engine.compress(input);
 */

// ══════════════════════════════════════════════
// Rule helpers
// ══════════════════════════════════════════════

function ci(pattern, replacement) {
  return { re: new RegExp(pattern, 'gi'), rep: replacement };
}
function ex(pattern, replacement) {
  return { re: new RegExp(pattern, 'gm'), rep: replacement };
}

// ══════════════════════════════════════════════
// OpenClaw-specific rules (always fire)
// ══════════════════════════════════════════════

const OPENCLAW_RULES = [
  ex('(?:user:\\s*)?Conversation info \\(untrusted metadata\\):\\s*```json\\s*\\{[\\s\\S]*?\\}\\s*```\\s*\\n?', ''),
  ex('```json\\s*\\{\\s*"message_id"[\\s\\S]*?\\}\\s*```\\s*\\n?', ''),
  ex('\\[\\[reply_to_current\\]\\]\\s*', ''),
  ex('\\[\\[reply_to_[^\\]]+\\]\\]\\s*', ''),
  ex('\\[\\[[^\\]]+\\]\\]\\s*', ''),
  ex('# Session: \\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2} UTC\\n[\\s\\S]*?## Conversation Summary\\n+', ''),
  ex('^- \\*\\*Session Key\\*\\*:.*\\n', ''),
  ex('^- \\*\\*Session ID\\*\\*:.*\\n', ''),
  ex('^- \\*\\*Source\\*\\*:.*\\n', ''),
  ex('^## Conversation Summary\\s*\\n', ''),
  ex('\\[\\w{2,4} \\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2} UTC\\]\\s*', ''),
  ex('(?:user: )?A new session was started via /new or /reset\\.[\\s\\S]*?what they want to do\\.\\s*\\n?', ''),
  ex('^assistant:\\s+', 'A: '),
  ex('^user:\\s+', 'U: '),
  ex('\\n{3,}', '\n\n'),
];

// ══════════════════════════════════════════════
// Level 0.2-0.4: Filler removal & normalization
// ══════════════════════════════════════════════

const FILLER_PHRASES = [
  ci('\\bplease\\s+note\\s+that\\b', ''),
  ci('\\bit\\s+is\\s+important\\s+to\\s+note\\s+that\\b', ''),
  ci('\\bit\\s+should\\s+be\\s+noted\\s+that\\b', ''),
  ci('\\bplease\\s+make\\s+sure\\s+to\\b', ''),
  ci('\\bplease\\s+ensure\\s+that\\b', 'ensure'),
  ci('\\bplease\\s+remember\\s+to\\b', ''),
  ci('\\bkindly\\b', ''),
  ci('\\bin\\s+order\\s+to\\b', 'to'),
  ci('\\bfor\\s+the\\s+purpose\\s+of\\b', 'for'),
  ci('\\bwith\\s+the\\s+goal\\s+of\\b', 'to'),
  ci('\\bdue\\s+to\\s+the\\s+fact\\s+that\\b', 'because'),
  ci('\\bin\\s+the\\s+event\\s+that\\b', 'if'),
  ci('\\bat\\s+this\\s+point\\s+in\\s+time\\b', 'now'),
  ci('\\bat\\s+the\\s+present\\s+time\\b', 'now'),
  ci('\\bin\\s+a\\s+manner\\s+that\\b', 'so that'),
  ci('\\bas\\s+a\\s+result\\s+of\\b', 'from'),
  ci('\\bprior\\s+to\\b', 'before'),
  ci('\\bsubsequent\\s+to\\b', 'after'),
  ci('\\bin\\s+the\\s+case\\s+that\\b', 'if'),
  ci('\\bfor\\s+the\\s+reason\\s+that\\b', 'because'),
  ci('\\bwith\\s+regard\\s+to\\b', 're:'),
  ci('\\bwith\\s+respect\\s+to\\b', 're:'),
  ci('\\bin\\s+regard\\s+to\\b', 're:'),
  ci('\\bin\\s+terms\\s+of\\b', 'for'),
  ci('\\bactually\\b', ''),
  ci('\\bbasically\\b', ''),
  ci('\\bessentially\\b', ''),
  ci('\\bfundamentally\\b', ''),
  ci('\\bliterally\\b', ''),
  ci('\\bobviously\\b', ''),
  ci('\\bclearly\\b', ''),
  ci('\\bneedless\\s+to\\s+say\\b', ''),
  ci('\\bit\\s+goes\\s+without\\s+saying\\b', ''),
  ci('\\bas\\s+you\\s+know\\b', ''),
  ci('\\bas\\s+we\\s+all\\s+know\\b', ''),
  ci('\\butilize\\b', 'use'),
  ci('\\butilization\\b', 'use'),
  ci('\\bimplement\\b', 'add'),
  ci('\\bimplementation\\b', 'adding'),
  ci('\\bfacilitate\\b', 'help'),
  ci('\\bdemonstrate\\b', 'show'),
  ci('\\bsubsequently\\b', 'then'),
  ci('\\bnevertheless\\b', 'but'),
  ci('\\bnonetheless\\b', 'but'),
  ci('\\bwith\\s+the\\s+exception\\s+of\\b', 'except'),
  ci('\\ba\\s+sufficient\\s+amount\\s+of\\b', 'enough'),
  ci('\\bfor\\s+example\\b', 'e.g.'),
  ci('\\bthat\\s+is\\s+to\\s+say\\b', 'i.e.'),
  ci('\\bthe\\s+following\\s+code\\s*:\\s*', 'Code: '),
  ci('\\bas\\s+shown\\s+below\\s*:\\s*', ''),
  ci('\\bas\\s+described\\s+below\\s*:\\s*', ''),
  ci('\\bthis\\s+means\\s+that\\b', 'meaning'),
  ci('\\bindicate\\b', 'show'),
];

const VERBOSE_PHRASES = [
  ci('\\bmake\\s+sure\\s+to\\b', 'ensure'),
  ci('\\btake\\s+into\\s+account\\b', 'consider'),
  ci('\\btake\\s+into\\s+consideration\\b', 'consider'),
  ci('\\bis\\s+able\\s+to\\b', 'can'),
  ci('\\bare\\s+able\\s+to\\b', 'can'),
  ci('\\bhas\\s+the\\s+ability\\s+to\\b', 'can'),
  ci('\\bhave\\s+the\\s+ability\\s+to\\b', 'can'),
  ci('\\bin\\s+addition\\s+to\\b', 'also'),
  ci('\\ba\\s+large\\s+number\\s+of\\b', 'many'),
  ci('\\ba\\s+significant\\s+number\\s+of\\b', 'many'),
  ci('\\bthe\\s+vast\\s+majority\\s+of\\b', 'most'),
  ci('\\bthe\\s+majority\\s+of\\b', 'most'),
  ci('\\bin\\s+close\\s+proximity\\s+to\\b', 'near'),
  ci('\\buntil\\s+such\\s+time\\s+as\\b', 'until'),
  ci('\\bduring\\s+the\\s+course\\s+of\\b', 'during'),
  ci('\\bon\\s+a\\s+daily\\s+basis\\b', 'daily'),
  ci('\\bon\\s+a\\s+regular\\s+basis\\b', 'regularly'),
  ci('\\bin\\s+the\\s+near\\s+future\\b', 'soon'),
  ci('\\bat\\s+the\\s+end\\s+of\\s+the\\s+day\\b', 'ultimately'),
  ci('\\bthe\\s+fact\\s+that\\b', 'that'),
  ci('\\bin\\s+light\\s+of\\b', 'given'),
  ci('\\bthere\\s+is\\s+a\\s+need\\s+to\\b', 'must'),
  ci('\\bit\\s+is\\s+necessary\\s+to\\b', 'must'),
  ci('\\bit\\s+is\\s+recommended\\s+that\\b', 'should'),
  ci('\\bit\\s+is\\s+suggested\\s+that\\b', 'should'),
  ci('\\bit\\s+is\\s+advisable\\s+to\\b', 'should'),
  ci('\\byour\\s+(?:primary\\s+)?(?:role|responsibility|job|task)\\s+is\\s+to\\b', ''),
  ci('\\byour\\s+goal\\s+is\\s+to\\b', ''),
  ci('\\byou\\s+are\\s+(?:designed|tasked|meant|intended)\\s+to\\b', ''),
  ci('\\byou\\s+are\\s+responsible\\s+for\\b', 'handle'),
  ci('\\bin\\s+your\\s+(?:responses?|analysis|review|recommendations?)\\b', ''),
  ci('\\bwhen\\s+(?:possible|appropriate|applicable|relevant|necessary)\\s*,?\\b', ''),
  ci('\\bat\\s+all\\s+times\\b', 'always'),
  ci('\\bwhenever\\s+possible\\b', ''),
  ci('\\bas\\s+(?:needed|required|appropriate|necessary)\\b', ''),
  ci('\\bwhen\\s+dealing\\s+with\\b', 'for'),
  ci('\\bwhen\\s+working\\s+with\\b', 'for'),
  ci('\\bwhen\\s+responding\\s+to\\b', 'for'),
  ci('\\bwhen\\s+providing\\b', 'for'),
  ci('\\bthorough\\s+and\\s+comprehensive\\b', 'thorough'),
  ci('\\bclear\\s+and\\s+concise\\b', 'concise'),
  ci('\\baccurate\\s+and\\s+up[- ]to[- ]date\\b', 'current'),
  ci('\\bhelpful\\s+and\\s+(?:friendly|supportive|informative)\\b', 'helpful'),
  ci('\\balways\\s+remember\\s+(?:that|to)\\b', ''),
  ci('\\balways\\s+make\\s+sure\\b', 'ensure'),
  ci('\\balways\\s+keep\\s+in\\s+mind\\b', 'note:'),
  ci('\\bplease\\s+provide\\b', 'provide'),
  ci('\\bplease\\s+include\\b', 'include'),
  ci('\\byou\\s+should\\s+also\\b', 'also'),
  ci('\\byou\\s+should\\s+always\\b', 'always'),
  ci('\\byou\\s+should\\s+never\\b', 'never'),
  ci('\\bwork\\s+diligently\\s+to\\b', ''),
  ci('\\bstep-by-step\\s+instructions\\b', 'steps'),
  ci('\\bstep-by-step\\s+(?:guide|process|procedure)\\b', 'steps'),
  ci('\\bincluding\\s+but\\s+not\\s+limited\\s+to\\b', 'including'),
  ci('\\bin\\s+(?:a|the)\\s+(?:clear|concise|timely|professional)\\s+(?:manner|way|fashion)\\b', 'clearly'),
  ci('\\bwith\\s+(?:a\\s+)?focus\\s+on\\b', 'focusing on'),
  ci('\\bregardless\\s+of\\s+(?:whether|how|what)\\b', 'regardless'),
  ci('\\bspecifically\\b', ''),
  ci('\\bparticularly\\b', ''),
  ci('\\b(?:ensure|verify|confirm|check|note)\\s+that\\b', '$1'),
  ci('\\bin\\s+the\\s+process\\s+of\\b', 'while'),
  ci('\\bby\\s+means\\s+of\\b', 'via'),
  ci('\\bin\\s+the\\s+absence\\s+of\\b', 'without'),
  ci('\\bto\\s+the\\s+best\\s+of\\s+your\\s+(?:ability|abilities|knowledge)\\b', ''),
];

// ══════════════════════════════════════════════
// Level 0.5-0.7: Structural compression
// ══════════════════════════════════════════════

const IMPERATIVE_CONVERSIONS = [
  ci('\\byou\\s+should\\b', ''),
  ci('\\byou\\s+must\\b', ''),
  ci('\\byou\\s+need\\s+to\\b', ''),
  ci('\\byou\\s+are\\s+required\\s+to\\b', ''),
  ci('\\byou\\s+are\\s+expected\\s+to\\b', ''),
  ci('\\bmake\\s+sure\\s+you\\b', ''),
  ci('\\bensure\\s+that\\s+you\\b', ''),
  ci('\\bremember\\s+to\\b', ''),
  ci('\\balways\\s+make\\s+sure\\s+to\\b', 'always'),
  ci('\\bbe\\s+sure\\s+to\\b', ''),
  ci('\\byou\\s+will\\s+(?:need\\s+to|want\\s+to|have\\s+to)\\b', ''),
  ci('\\bit\\s+is\\s+(?:important|essential|critical|crucial|vital)\\s+(?:that\\s+you|to)\\b', ''),
  ci('\\bkeep\\s+in\\s+mind\\s+that\\b', ''),
  ci('\\bbe\\s+(?:mindful|aware|careful)\\s+(?:of|that|to)\\b', ''),
  ci('\\bpay\\s+(?:attention|close\\s+attention)\\s+to\\b', 'note'),
  ci('\\bstrive\\s+to\\b', ''),
  ci('\\balways\\s+follow\\b', 'follow'),
  ci('\\balways\\s+prioritize\\b', 'prioritize'),
  ci('\\balways\\s+verify\\b', 'verify'),
  ci('\\balways\\s+check\\b', 'check'),
  ci('\\balways\\s+include\\b', 'include'),
  ci('\\balways\\s+ensure\\b', 'ensure'),
  ci('\\balways\\s+provide\\b', 'provide'),
  ci('\\balways\\s+consider\\b', 'consider'),
  ci('\\balways\\s+use\\b', 'use'),
  ci('\\balways\\s+maintain\\b', 'maintain'),
  ci('\\byou\\s+are\\s+a\\b', 'Act as'),
  ci('\\byou\\s+are\\s+an\\b', 'Act as'),
  ci('\\bin\\s+addition\\s*,?\\s*\\b', 'Also '),
  ci('\\bfurthermore\\s*,?\\s*\\b', 'Also '),
  ci('\\bmoreover\\s*,?\\s*\\b', 'Also '),
  ci('\\badditionally\\s*,?\\s*\\b', 'Also '),
];

const CLAUSE_COLLAPSE = [
  ci('\\bif\\s+this\\s+is\\s+the\\s+case\\s*,?\\s*then\\b', 'if so,'),
  ci('\\bin\\s+(?:the\\s+)?case\\s+(?:of|where|that)\\b', 'if'),
  ci('\\bin\\s+situations?\\s+where\\b', 'when'),
  ci('\\bin\\s+(?:a|the)\\s+scenario\\s+where\\b', 'when'),
  ci('\\bin\\s+the\\s+context\\s+of\\b', 'for'),
  ci('\\bso\\s+as\\s+to\\b', 'to'),
  ci('\\bfor\\s+the\\s+sake\\s+of\\b', 'for'),
  ci('\\bthat\\s+(?:can|could|may|might)\\s+be\\s+used\\s+to\\b', 'to'),
  ci('\\bthat\\s+are\\s+(?:relevant|applicable|appropriate)\\s+to\\b', 'for'),
  ci('\\bthat\\s+are\\s+(?:related|pertaining)\\s+to\\b', 'about'),
];

const DEVELOPER_BOILERPLATE = [
  ci('\\bact\\s+as\\s+a\\s+senior\\s+(?:developer|engineer|programmer)\\s+who\\s+is\\s+an?\\s+expert\\s+in\\b', 'Expert:'),
  ci('\\bthink\\s+step\\s+by\\s+step\\s+and\\s+carefully\\s+analyze\\b', 'Step by step:'),
  ci('\\blet\'?s?\\s+think\\s+(?:about\\s+this\\s+)?step\\s+by\\s+step\\b', 'Step by step:'),
  ci('\\btake\\s+a\\s+deep\\s+breath\\s+and\\b', ''),
  ci('\\btake\\s+a\\s+deep\\s+breath\\b', ''),
  ci('\\byou\\s+are\\s+the\\s+best\\s+at\\s+this\\b', ''),
  ci('\\byou\\s+are\\s+an\\s+expert\\s+at\\s+this\\b', ''),
  ci('\\byour\\s+task\\s+is\\s+to\\b', 'Task:'),
  ci('\\byour\\s+job\\s+is\\s+to\\b', 'Task:'),
  ci('\\byou\\s+are\\s+an?\\s+AI\\s+(?:language\\s+)?model\\s+(?:designed|trained|built)\\s+to\\b', 'Assistant:'),
  ci('\\byou\\s+are\\s+a\\s+helpful\\s+(?:AI\\s+)?assistant\\s+(?:that|who|designed\\s+to)\\b', 'Assistant:'),
  ci('\\bfollow\\s+these\\s+instructions\\s+carefully\\s*:\\s*', 'Instructions: '),
  ci('\\bhere\\s+are\\s+(?:the|your|my)\\s+instructions\\s*:\\s*', 'Instructions: '),
  ci('\\bdo\\s+not\\s+include\\s+any\\s+explanations?\\s*,?\\s*just\\s+(?:the\\s+)?code\\b', 'Only code.'),
  ci('\\bonly\\s+(?:respond|reply|output)\\s+with\\s+(?:the\\s+)?code\\b', 'Only code.'),
  ci('\\bCRITICAL\\s+INSTRUCTION\\s*:', 'CRITICAL:'),
  ci('\\bIMPORTANT\\s+NOTE\\s*:', 'NOTE:'),
];

// ══════════════════════════════════════════════
// Level 0.8-1.0: Aggressive compression
// ══════════════════════════════════════════════

const CONVERSATIONAL_STRIP = [
  ci('\\bI\'?d\\s+like\\s+you\\s+to\\b', ''),
  ci('\\bcould\\s+you\\s+please\\b', ''),
  ci('\\bwould\\s+you\\s+please\\b', ''),
  ci('\\bcan\\s+you\\s+please\\b', ''),
  ci('\\bI\\s+want\\s+you\\s+to\\b', ''),
  ci('\\bI\\s+need\\s+you\\s+to\\b', ''),
  ci('\\bif\\s+possible\\s*,?\\b', ''),
  ci('\\bthis\\s+is\\s+very\\s+important\\s*[.!]?\\b', ''),
  ci('\\bprovide\\s+(?:detailed|comprehensive|thorough)\\s+(?:explanations?|responses?|analysis)\\b', 'explain thoroughly'),
  ci('\\bprovide\\s+(?:clear|concise|helpful)\\s+(?:explanations?|responses?|information)\\b', 'explain clearly'),
  ci('\\brecognizing\\s+that\\b', 'since'),
  ci('\\bgiven\\s+the\\s+fact\\s+that\\b', 'since'),
  ci('\\byou\\s+are\\s+(?:a|an)\\s+(?:experienced|expert|knowledgeable|skilled|seasoned|senior)\\b', 'you are a'),
  ci('\\bwith\\s+(?:over\\s+)?\\d+\\s+years?\\s+of\\s+(?:experience|expertise)(?:\\s+in\\s+(?:the\\s+)?field)?\\b', ''),
  ci('\\bprovide\\s+(?:personalized|tailored|custom)\\s+(?:recommendations?|suggestions?|guidance|advice)\\b', 'recommend'),
  ci('\\bprovide\\s+(?:practical|useful|helpful|actionable)\\s+(?:tips?|advice|guidance|suggestions?)\\b', 'advise'),
  ci('\\bshould\\s+not\\s+be\\s+(?:considered|used)\\s+as\\s+(?:a\\s+substitute|replacement)\\s+for\\b', 'is not'),
  ci('\\bfor\\s+(?:educational|informational)\\s+purposes\\s+only\\b', 'educational only'),
  ci('\\b(?:consult|speak)\\s+with\\s+(?:a\\s+)?(?:qualified|licensed|certified)?\\s*(?:professional|advisor|attorney|doctor)\\s+before\\b', 'consult a professional before'),
  ci('\\b(?:maintain|keep)\\s+(?:a\\s+)?(?:professional|respectful|friendly|positive)\\s+(?:tone|demeanor|attitude)\\b', 'be professional'),
];

const AI_OUTPUT_NOISE = [
  ci('\\bas\\s+an?\\s+AI\\s+(?:language\\s+)?model\\s*,?\\s*(?:I\\s+)?', ''),
  ci('\\bas\\s+an?\\s+(?:artificial\\s+intelligence|AI)\\s*,?\\s*', ''),
  ci('\\bI\\s+(?:am|\'m)\\s+sorry\\s+for\\s+(?:the|any)\\s+confusion\\s*[.,]?\\s*', ''),
  ci('\\bI\\s+apologize\\s+for\\s+(?:the|any)\\s+confusion\\s*[.,]?\\s*', ''),
  ci('\\bI\\s+apologize\\s+for\\s+(?:the|my)\\s+oversight\\s*[.,]?\\s*', ''),
  ci('\\bhowever\\s*,?\\s*it\'?s?\\s+important\\s+to\\s+(?:consider|note|remember)\\b', 'Note:'),
  ci('\\bit\'?s?\\s+worth\\s+(?:noting|mentioning|considering)\\s+that\\b', 'Note:'),
  ci('\\blet\\s+me\\s+know\\s+if\\s+you\\s+need\\s+any\\s+(?:further|more|additional)\\s+(?:help|assistance|information)\\s*[.!]?\\s*', ''),
  ci('\\bdon\'?t\\s+hesitate\\s+to\\s+(?:ask|reach\\s+out)\\s*[.!]?\\s*', ''),
  ci('\\bI\\s+hope\\s+this\\s+helps\\s*[.!]?\\s*', ''),
  ci('\\bfeel\\s+free\\s+to\\s+ask\\s+if\\s+you\\s+have\\s+(?:any\\s+)?(?:more|other|further)\\s+questions?\\s*[.!]?\\s*', ''),
  ci('\\bif\\s+you\\s+have\\s+any\\s+(?:other|more|further)\\s+questions?\\s*,?\\s*(?:feel\\s+free\\s+to\\s+ask|let\\s+me\\s+know)\\s*[.!]?\\s*', ''),
];

const MARKDOWN_MINIFICATION = [
  ex('^#{4,6}\\s+', '# '),
  ex('^###\\s+', '## '),
  ex('\\*{2}([^*]+)\\*{2}', '$1'),
  ex('\\|\\s{2,}', '| '),
  ex('\\s{2,}\\|', ' |'),
  ex('<br\\s*/?>\\s*', '\n'),
  ex('</?(?:b|strong)>', ''),
  ex('</?(?:i|em)>', ''),
  ex('^\\s*(?:---+|___+|\\*\\*\\*+)\\s*$', ''),
  ex('```(?:python|javascript|typescript|bash|shell|sh|json|yaml|yml|xml|html|css|sql)\\s*\\n', '```\n'),
];

const SOURCE_CODE_COMPRESSION = [
  ex('^\\s*//(?!\\s*TODO)[^\\n]{2,}$', ''),
  ex('^\\s*#\\s+(?!TODO|!|\\s*$)[^\\n]{3,}$', ''),
  ex('^\\s*\\n(?=\\s*\\n)', ''),
];

const CONTEXT_DEDUPLICATION = [
  ex('^(?:Thanks!?|Thank you!?|Got it!?|OK!?|Okay!?|Great!?|Perfect!?|Understood!?)\\s*$', ''),
  ex('data:[a-zA-Z/]+;base64,[A-Za-z0-9+/=]{100,}', '[BASE64_TRUNCATED]'),
];

const SEMANTIC_FORMATTING = [
  ci('\\bten\\s+thousand\\b', '10k'),
  ci('\\bone\\s+hundred\\s+thousand\\b', '100k'),
  ci('\\bone\\s+million\\b', '1M'),
  ci('\\bone\\s+billion\\b', '1B'),
  ci('\\bone\\s+thousand\\b', '1k'),
  ci('\\bdo\\s+not\\b', "don't"),
  ci('\\bcannot\\b', "can't"),
  ci('\\bwill\\s+not\\b', "won't"),
  ci('\\bshould\\s+not\\b', "shouldn't"),
  ci('\\bwould\\s+not\\b', "wouldn't"),
  ci('\\bcould\\s+not\\b', "couldn't"),
  ci('\\bdoes\\s+not\\b', "doesn't"),
  ci('\\bdid\\s+not\\b', "didn't"),
  ci('\\bis\\s+not\\b', "isn't"),
  ci('\\bare\\s+not\\b', "aren't"),
  ci('\\bhas\\s+not\\b', "hasn't"),
  ci('\\bhave\\s+not\\b', "haven't"),
  ci('[?&]utm_\\w+=[^&\\s)]*', ''),
  ci('[?&](?:ref|source|campaign|medium|fbclid|gclid)=[^&\\s)]*', ''),
];

const ANTI_NOISE = [
  ex('(?:[A-Z]:\\\\(?:Users|Program Files)\\\\[^\\s]*\\\\)', '.../'),
  ex('/(?:home|usr|var|opt)/[^\\s/]+(?:/[^\\s/]+){2,}/', '.../'),
  ex('/node_modules/[^\\s]+/', '[node_modules]/ '),
  { re: /\x1b\[[0-9;]*m/g, rep: '' },
  ex('<thought>\\s*</thought>', ''),
  ex('<thinking>\\s*</thinking>', ''),
  ex('<scratchpad>\\s*</scratchpad>', ''),
];

const CREDENTIAL_STRIP = [
  ci('\\bwith\\s+(?:over\\s+|more\\s+than\\s+)?\\d+\\+?\\s+years?\\s+(?:of\\s+)?experience(?:\\s+in\\s+[\\w\\s]+?)?(?=\\.)', ''),
  ci('\\bwith\\s+extensive\\s+(?:knowledge|experience)\\s+(?:of|in)\\s+[\\w\\s]+(?=\\.)', ''),
  ci('\\bdesigned\\s+to\\s+(?:help|assist|support)\\b', 'helps'),
];

const DISCLAIMER_COLLAPSE = [
  ci('(?:Please\\s+note\\s+that\\s+)?(?:your|this)\\s+(?:analysis|information|response)\\s+should\\s+not\\s+be\\s+considered\\s+(?:as\\s+)?(?:personalized\\s+)?(?:legal|financial|medical|professional)\\s+advice[^.]*\\.', '[Not professional advice.]'),
  ci('(?:They|Users?|You)\\s+should\\s+(?:always\\s+)?consult\\s+with\\s+(?:a\\s+|their\\s+)?(?:licensed\\s+|qualified\\s+)?(?:financial\\s+advisor|legal\\s+counsel|healthcare\\s+professional|doctor|attorney)[^.]*\\.', '[Consult a professional.]'),
  ci('Always\\s+(?:encourage|remind)\\s+users?\\s+to\\s+consult\\s+with\\s+(?:qualified\\s+)?(?:healthcare\\s+)?professionals?[^.]*\\.', ''),
];

const ADJECTIVE_COLLAPSE = [
  ci('\\btechnical\\s+issues,?\\s+billing\\s+questions,?\\s+and\\s+product\\s+inquiries\\b', 'technical/billing/product issues'),
  ci('\\berror\\s+handling,?\\s+edge\\s+cases,?\\s+input\\s+validation,?\\s+and\\s+resource\\s+management\\b', 'errors/edge-cases/validation/resources'),
  ci('\\bstep-by-step\\s+instructions\\b', 'steps'),
  ci('\\bpractical\\s+examples\\s+and\\s+real-world\\s+use\\s+cases\\b', 'examples'),
  ci('\\b(?:clear|detailed)\\s+(?:explanations?|commentary)\\s+(?:on|that|for)\\b', 'explaining'),
];

const CLAUSE_SIMPLIFY = [
  ci('\\bwithout\\s+getting\\s+overly\\s+\\w+\\b', 'concisely'),
  ci('\\bwhile\\s+(?:remaining|staying|being)\\s+\\w+\\s+and\\s+\\w+\\b', ''),
  ci('\\bbefore\\s+making\\s+any\\s+(?:decisions?|changes?|modifications?)\\b', 'first'),
  ci('\\bdiversity,?\\s+equity,?\\s+and\\s+inclusion\\s*(?:principles)?\\b', 'DEI'),
  ci('\\bbenefits,?\\s+risks,?\\s+and\\s+(?:alternatives|side\\s+effects)\\b', 'tradeoffs'),
  ci('\\bwhile\\s+(?:still\\s+)?(?:maintaining|ensuring|preserving)\\s+(\\w+(?:\\s+\\w+)?)\\b', '(keep $1)'),
  ci('\\bnot\\s+(?:just|only)\\s+(\\w+(?:\\s+\\w+)?)\\s+but\\s+(?:also\\s+)?(\\w+)\\b', '$1 and $2'),
];

const ADVERB_STRIP = [
  ci('\\b(?:very|extremely|highly|incredibly|particularly|especially|really|truly)\\s+', ''),
  ci('\\bwork\\s+(?:diligently|carefully|hard|tirelessly)\\b', 'work'),
  ci('\\bimmediately\\s+actionable\\b', 'actionable'),
  ci('\\bon\\s+this\\s+(?:topic|matter|issue|subject)\\b', ''),
];

// ══════════════════════════════════════════════
// Engine
// ══════════════════════════════════════════════

function applyRules(text, rules) {
  let result = text;
  let hits = 0;
  for (const rule of rules) {
    const before = result;
    result = result.replace(rule.re, rule.rep);
    if (result !== before) hits++;
  }
  return { text: result, hits };
}

function cleanupWhitespace(text) {
  return text
    .replace(/  +/g, ' ')
    .replace(/ ([.,;:!?])/g, '$1')
    .replace(/\n +\n/g, '\n\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

class NyquestCompressor {
  constructor(level = 0.7) {
    this.level = Math.max(0, Math.min(1, level));
    this.totalHits = 0;
  }

  _apply(text, rules) {
    const r = applyRules(text, rules);
    this.totalHits += r.hits;
    return r.text;
  }

  compress(text) {
    if (this.level === 0) return { text, stats: { totalHits: 0 } };

    this.totalHits = 0;
    let result = text;

    // Always: OpenClaw metadata
    result = this._apply(result, OPENCLAW_RULES);

    // Tier 1: 0.2+ filler + verbose
    if (this.level >= 0.2) {
      result = this._apply(result, FILLER_PHRASES);
      result = this._apply(result, VERBOSE_PHRASES);
    }

    // Tier 2: 0.5+ structural
    if (this.level >= 0.5) {
      result = this._apply(result, IMPERATIVE_CONVERSIONS);
      result = this._apply(result, CLAUSE_COLLAPSE);
      result = this._apply(result, DEVELOPER_BOILERPLATE);
      result = this._apply(result, SEMANTIC_FORMATTING);
      result = this._apply(result, CREDENTIAL_STRIP);
    }

    // Tier 3: 0.8+ aggressive
    if (this.level >= 0.8) {
      result = this._apply(result, CONVERSATIONAL_STRIP);
      result = this._apply(result, AI_OUTPUT_NOISE);
      result = this._apply(result, MARKDOWN_MINIFICATION);
      result = this._apply(result, SOURCE_CODE_COMPRESSION);
      result = this._apply(result, CONTEXT_DEDUPLICATION);
      result = this._apply(result, ANTI_NOISE);
      result = this._apply(result, DISCLAIMER_COLLAPSE);
      result = this._apply(result, ADJECTIVE_COLLAPSE);
      result = this._apply(result, CLAUSE_SIMPLIFY);
      result = this._apply(result, ADVERB_STRIP);
    }

    result = cleanupWhitespace(result);

    return {
      text: result,
      stats: { totalHits: this.totalHits }
    };
  }
}

// Export for content script (globalThis) and module (export) contexts
if (typeof globalThis !== 'undefined') {
  globalThis.NyquestCompressor = NyquestCompressor;
}
try { if (typeof module !== 'undefined') module.exports = NyquestCompressor; } catch(_) {}
