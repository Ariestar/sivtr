pub mod index;
pub mod model;
pub mod refs;

pub use index::{
    work_record_content_matches, WorkRecordIndex, WorkRecordMatch, WorkRecordSearchScope,
};
pub use model::{
    chat_turn_ranges, is_real_user_block, ChatMessage, RecordText, RecordTextMode, WorkChannel,
    WorkOutcome, WorkPart, WorkPartIo, WorkPartKind, WorkPayload, WorkRecord, WorkRecordCopyParts,
    WorkRecordKind, WorkSessionRef, WorkSource, WorkStatus, WorkText, WorkTime,
    RECORD_SCHEMA_VERSION,
};
pub use refs::{WorkRef, WorkRefSelector, WorkRefTarget};
