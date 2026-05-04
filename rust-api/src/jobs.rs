use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 3D printer model types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrinterModel {
    A1,
    A1Mini,
    P1P,
    P1S,
    X1C,
    X1E,
}

impl PrinterModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::A1 => "A1",
            Self::A1Mini => "A1 Mini",
            Self::P1P => "P1P",
            Self::P1S => "P1S",
            Self::X1C => "X1C",
            Self::X1E => "X1E",
        }
    }
}

/// Print job status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    InProgress,
    Printing,
    Collect,
    Completed,
    Cancelled,
    Error,
}

/// Print job in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintJob {
    pub id: String,
    pub student_name: String,
    pub class_period: String,
    pub teacher: String,
    pub filename: String,
    pub printer_model: PrinterModel,
    pub printer_id: Option<String>,
    pub file_path: String,
    pub status: JobStatus,
    pub progress_percent: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the job was marked completed
    pub completed_at: Option<DateTime<Utc>>,
}

/// Job queue manager
pub struct JobQueue {
    jobs: Arc<RwLock<HashMap<String, PrintJob>>>,
    job_order: Arc<RwLock<Vec<String>>>, // Track order of queued jobs
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            job_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Submit a new print job
    pub async fn submit_job(
        &self,
        student_name: String,
        class_period: String,
        teacher: String,
        filename: String,
        printer_model: PrinterModel,
        file_path: String,
    ) -> Result<PrintJob, String> {
        // Validate filename
        if filename.is_empty() || filename.len() > 255 {
            return Err("Filename must be 1-255 characters".to_string());
        }

        // Validate student name
        if student_name.is_empty() || student_name.len() > 100 {
            return Err("Student name must be 1-100 characters".to_string());
        }

        // Validate class period
        if class_period.is_empty() || class_period.len() > 50 {
            return Err("Class period must be 1-50 characters".to_string());
        }

        // Validate teacher
        if teacher.is_empty() || teacher.len() > 100 {
            return Err("Teacher must be 1-100 characters".to_string());
        }

        let job = PrintJob {
            id: uuid_simple(),
            student_name,
            class_period,
            teacher,
            filename,
            printer_model,
            printer_id: None,
            file_path,
            status: JobStatus::Queued,
            progress_percent: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
        };

        let job_id = job.id.clone();
        self.jobs.write().await.insert(job_id.clone(), job.clone());
        self.job_order.write().await.push(job_id);

        Ok(job)
    }

    /// Get job by ID
    pub async fn get_job(&self, job_id: &str) -> Option<PrintJob> {
        self.jobs.read().await.get(job_id).cloned()
    }

    /// List all jobs
    pub async fn list_jobs(&self) -> Vec<PrintJob> {
        self.jobs.read().await.values().cloned().collect()
    }

    /// List jobs in queue (ordered)
    pub async fn list_queued_jobs(&self) -> Vec<PrintJob> {
        let jobs = self.jobs.read().await;
        let job_order = self.job_order.read().await;
        job_order
            .iter()
            .filter_map(|id| jobs.get(id).cloned())
            .filter(|j| j.status == JobStatus::Queued)
            .collect()
    }

    /// Dispatch a job to a printer
    pub async fn dispatch_job(&self, job_id: &str, printer_id: String) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        if job.status != JobStatus::Queued {
            return Err(format!("Job is {}, cannot dispatch", format!("{:?}", job.status).to_lowercase()));
        }

        job.printer_id = Some(printer_id);
        job.status = JobStatus::InProgress;
        job.updated_at = Utc::now();

        // Remove from queue order
        drop(jobs);
        let mut order = self.job_order.write().await;
        order.retain(|id| id != job_id);

        let jobs = self.jobs.read().await;
        Ok(jobs.get(job_id).unwrap().clone())
    }

    #[allow(dead_code)]
    /// Update job progress
    pub async fn update_progress(&self, job_id: &str, progress: u32) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        job.progress_percent = progress.min(100);
        job.updated_at = Utc::now();

        Ok(job.clone())
    }

    /// Mark job as completed
    pub async fn complete_job(&self, job_id: &str) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        job.status = JobStatus::Completed;
        job.progress_percent = 100;
        job.completed_at = Some(Utc::now());
        job.updated_at = Utc::now();

        // Remove from queue order if still there
        let jid = job_id.to_string();
        drop(jobs);
        let mut order = self.job_order.write().await;
        order.retain(|id| id != &jid);

        let jobs = self.jobs.read().await;
        Ok(jobs.get(job_id).unwrap().clone())
    }

    /// Delete a completed job
    pub async fn delete_job(&self, job_id: &str) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        if job.status != JobStatus::Completed {
            return Err(format!(
                "Can only delete completed jobs, this is {}",
                format!("{:?}", job.status).to_lowercase()
            ));
        }

        let removed = jobs.remove(job_id).unwrap();

        // Remove from queue order
        drop(jobs);
        let mut order = self.job_order.write().await;
        order.retain(|id| id != job_id);

        Ok(removed)
    }

    #[allow(dead_code)]
    /// Mark job as error
    pub async fn error_job(&self, job_id: &str) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        job.status = JobStatus::Error;
        job.updated_at = Utc::now();

        Ok(job.clone())
    }

    /// Find the in-progress job assigned to a specific printer
    pub async fn job_for_printer(&self, printer_id: &str) -> Option<PrintJob> {
        let jobs = self.jobs.read().await;
        jobs.values()
            .find(|j| j.printer_id.as_deref() == Some(printer_id) && j.status == JobStatus::InProgress)
            .cloned()
    }

    /// Update a job's file path (used after renaming the uploaded file)
    pub async fn update_file_path(&self, job_id: &str, new_path: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(job_id) {
            job.file_path = new_path;
            job.updated_at = Utc::now();
        }
    }

    /// Cancel a job
    pub async fn cancel_job(&self, job_id: &str) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        if job.status != JobStatus::Queued {
            return Err(format!(
                "Can only cancel queued jobs, this is {}",
                format!("{:?}", job.status).to_lowercase()
            ));
        }

        job.status = JobStatus::Cancelled;
        job.updated_at = Utc::now();

        // Remove from queue order
        drop(jobs);
        let mut order = self.job_order.write().await;
        order.retain(|id| id != job_id);

        let jobs = self.jobs.read().await;
        Ok(jobs.get(job_id).unwrap().clone())
    }

    /// Set job status (admin-controlled status updates)
    pub async fn set_job_status(&self, job_id: &str, new_status: JobStatus) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        // Handle Collect → Complete auto-transition
        let final_status = if new_status == JobStatus::Collect && job.status != JobStatus::Collect {
            // Mark as Collect (waiting for collection), but doesn't auto-complete yet
            JobStatus::Collect
        } else if new_status == JobStatus::Completed && job.status == JobStatus::Collect {
            // Already Collect, now completing
            job.completed_at = Some(Utc::now());
            JobStatus::Completed
        } else {
            new_status
        };

        job.status = final_status.clone();
        job.updated_at = Utc::now();

        // If marking as Completed, remove from queue order
        if final_status == JobStatus::Completed {
            let jid = job_id.to_string();
            drop(jobs);
            let mut order = self.job_order.write().await;
            order.retain(|id| id != &jid);
            let jobs = self.jobs.read().await;
            Ok(jobs.get(job_id).unwrap().clone())
        } else {
            Ok(job.clone())
        }
    }

    /// Set printer for a job (for Printing status)
    pub async fn set_job_printer(&self, job_id: &str, printer_id: String) -> Result<PrintJob, String> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found".to_string())?;

        job.printer_id = Some(printer_id);
        job.updated_at = Utc::now();

        Ok(job.clone())
    }

    /// Clear (delete) all jobs with a specific status
    pub async fn clear_jobs_by_status(&self, status: JobStatus) -> Result<Vec<PrintJob>, String> {
        let mut jobs = self.jobs.write().await;
        let to_delete: Vec<String> = jobs
            .iter()
            .filter(|(_, j)| j.status == status)
            .map(|(id, _)| id.clone())
            .collect();

        let mut deleted_jobs = Vec::new();
        for job_id in to_delete {
            if let Some(job) = jobs.remove(&job_id) {
                deleted_jobs.push(job);
            }
        }

        // Remove from queue order
        drop(jobs);
        let mut order = self.job_order.write().await;
        order.retain(|id| {
            let jobs = futures::executor::block_on(self.jobs.read());
            jobs.contains_key(id)
        });

        Ok(deleted_jobs)
    }

    /// Clear all completed jobs
    pub async fn clear_completed_jobs(&self) -> Result<Vec<PrintJob>, String> {
        self.clear_jobs_by_status(JobStatus::Completed).await
    }
}

/// Generate simple UUID
pub fn uuid_simple() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
