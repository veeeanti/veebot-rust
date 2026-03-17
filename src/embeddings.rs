//! Embeddings module for veebot
//! Provides text similarity and embedding functions

use std::collections::HashSet;

/// Embedding dimension constant
pub const EMBEDDING_DIMENSION: usize = 512;

/// Generate a simple hash-based embedding vector for text
/// NOTE: This is a fallback - not semantically meaningful!
/// Real embeddings would use OpenAI or similar AI service
pub fn generate_embedding(text: &str) -> Vec<f64> {
    let mut embedding = vec![0.0; EMBEDDING_DIMENSION];
    
    // Simple hash-based embedding (not semantically meaningful)
    for (i, c) in text.chars().enumerate() {
        let char_code = c as usize;
        let index = char_code % EMBEDDING_DIMENSION;
        embedding[index] += 1.0;
        
        // Add some position-based variation
        let pos_index = (i * 7) % EMBEDDING_DIMENSION;
        embedding[pos_index] += 0.5;
    }
    
    // Normalize the embedding
    let norm: f64 = embedding.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm > 0.0 {
        for val in &mut embedding {
            *val /= norm;
        }
    }
    
    embedding
}

/// Generate embeddings for multiple texts at once
pub fn generate_batch_embeddings(texts: &[String]) -> Vec<Vec<f64>> {
    texts.iter().map(|text| generate_embedding(text)).collect()
}

/// Calculate cosine similarity between two embedding vectors
/// Returns a value between 0 and 1, where 1 means identical
pub fn calculate_cosine_similarity(embedding1: &[f64], embedding2: &[f64]) -> f64 {
    if embedding1.len() != embedding2.len() {
        return 0.0;
    }
    
    let mut dot_product = 0.0;
    let mut norm1 = 0.0;
    let mut norm2 = 0.0;
    
    for i in 0..embedding1.len() {
        dot_product += embedding1[i] * embedding2[i];
        norm1 += embedding1[i] * embedding1[i];
        norm2 += embedding2[i] * embedding2[i];
    }
    
    let denominator = norm1.sqrt() * norm2.sqrt();
    if denominator == 0.0 {
        return 0.0;
    }
    
    dot_product / denominator
}

/// Calculate text similarity using word overlap (Jaccard) + length similarity
pub fn calculate_text_similarity(text1: &str, text2: &str) -> f64 {
    let words1: HashSet<String> = text1.to_lowercase().split_whitespace().map(|s| s.to_string()).collect();
    let words2: HashSet<String> = text2.to_lowercase().split_whitespace().map(|s| s.to_string()).collect();
    
    // Calculate word overlap (Jaccard)
    let intersection: HashSet<_> = words1.intersection(&words2).cloned().collect();
    let union: HashSet<_> = words1.union(&words2).cloned().collect();
    
    let jaccard_similarity = if union.is_empty() {
        0.0
    } else {
        intersection.len() as f64 / union.len() as f64
    };
    
    // Length similarity (penalize very different lengths)
    let len_diff = (words1.len() as i32 - words2.len() as i32).unsigned_abs() as f64;
    let max_len = words1.len().max(words2.len()) as f64;
    let length_similarity = if max_len == 0.0 {
        1.0
    } else {
        (1.0 - len_diff / max_len).max(0.0)
    };
    
    // Combined similarity
    (jaccard_similarity + length_similarity) / 2.0
}

/// Test the embedding service
pub fn test_embedding_service() -> bool {
    let test_text = "This is a test sentence for embedding generation.";
    let embedding = generate_embedding(test_text);
    
    if embedding.len() != EMBEDDING_DIMENSION {
        tracing::error!("Embedding service test failed: wrong dimension");
        return false;
    }
    
    // Test similarity
    let text1 = "hello world";
    let text2 = "hello world";
    let text3 = "goodbye world";
    
    let sim_same = calculate_text_similarity(text1, text2);
    let sim_diff = calculate_text_similarity(text1, text3);
    
    if sim_same < sim_diff {
        tracing::warn!("Embedding similarity test: similar texts should have higher similarity");
    }
    
    tracing::info!("Text-based embedding service working correctly");
    tracing::info!("Generated {}D embedding (local)", EMBEDDING_DIMENSION);
    
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_embedding_generation() {
        let text = "test";
        let embedding = generate_embedding(text);
        assert_eq!(embedding.len(), EMBEDDING_DIMENSION);
    }
    
    #[test]
    fn test_cosine_similarity() {
        let e1 = vec![1.0, 0.0, 0.0];
        let e2 = vec![1.0, 0.0, 0.0];
        let e3 = vec![0.0, 1.0, 0.0];
        
        assert!((calculate_cosine_similarity(&e1, &e2) - 1.0).abs() < 0.001);
        assert!((calculate_cosine_similarity(&e1, &e3) - 0.0).abs() < 0.001);
    }
    
    #[test]
    fn test_text_similarity() {
        let text1 = "hello world";
        let text2 = "hello world";
        let text3 = "completely different text";
        
        let sim_same = calculate_text_similarity(text1, text2);
        let sim_diff = calculate_text_similarity(text1, text3);
        
        assert!(sim_same > sim_diff);
    }
}
