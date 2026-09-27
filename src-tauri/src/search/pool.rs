//! Generic goals used to calibrate semantic scores for whichever embedding
//! model is installed (see [`super::vectors`]).
//!
//! They are deliberately everyday requests unrelated to any particular Spark
//! library: a Spark that is close to many of them is written in generic
//! language and gets its similarity corrected accordingly. Changing this list
//! changes calibration; the semantic regression suite pins its behaviour.

pub const GENERIC_GOALS: &[&str] = &[
    "write a cover letter for a job application",
    "summarize a long article for me",
    "translate an email into Spanish",
    "plan a weekly workout routine",
    "draft a marketing email for a product launch",
    "explain a hard concept to a beginner",
    "brainstorm names for a new company",
    "create a lesson plan for a class",
    "convert this data into a table",
    "analyze customer feedback for common themes",
    "prepare me for a job interview",
    "write a blog post outline",
    "improve the tone of my message",
    "create a monthly budget",
    "generate social media captions",
    "outline a chapter of my novel",
    "fix the grammar in my essay",
    "compare two laptops before I buy one",
    "draft a clause for a contract",
    "write a product description",
    "improve my website's search ranking",
    "plan a trip itinerary",
    "write a speech for a wedding",
    "explain what this error message means",
    "negotiate a salary increase",
    "write a thank-you note",
    "create a weekly meal plan",
    "write song lyrics about summer",
    "make a presentation outline",
    "draft a press release",
    "help me learn a new language",
    "write a polite complaint letter",
    "create a to-do list for moving house",
    "give feedback on my resume",
    "suggest gift ideas",
    "write a short story for kids",
    "explain my medical test results in plain words",
    "create flashcards for studying",
    "draft a newsletter",
    "answer customer support emails",
];
