pub mod index;
pub mod model;
pub mod refs;
pub mod similarity;

pub use index::{
    work_record_content_matches, WorkRecordIndex, WorkRecordMatch, WorkRecordSearchScope,
};
pub use model::{
    chat_turn_ranges, is_real_user_block, RecordText, RecordTextMode, WorkChannel, WorkOutcome,
    WorkPart, WorkPartIo, WorkPartKind, WorkRecord, WorkRecordCopyParts, WorkRecordKind,
    WorkSessionRef, WorkSource, WorkStatus, WorkTime, RECORD_SCHEMA_VERSION,
};
pub use refs::{
    PartRangeSelector, WorkLink, WorkLinkKind, WorkRef, WorkRefSelector, WorkRefTarget,
};
pub use similarity::{semantic_search, SimilarityMatch};
